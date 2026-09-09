#[test]
fn contextual_callback_may_ignore_supplied_parameters() {
    lower(
        r#"async function main(): Promise<void> {
            await new Promise<number>((resolve) => resolve(1)).then(() => {});
        }"#,
    );
}

#[test]
fn contextual_void_callback_may_ignore_its_return_value() {
    lower(
        r#"declare function returnsJson(): Json;
        async function main(): Promise<void> {
            await new Promise<void>((resolve) => returnsJson());
        }"#,
    );
}

#[test]
fn contextual_void_callback_may_discard_a_block_return_value() {
    lower(
        r#"declare function consume(callback: () => void): void;
        function main(): void {
            consume((): boolean => { return true; });
        }"#,
    );
}

#[test]
fn timer_callback_may_capture_its_own_handle() {
    lower(
        r#"function main(): void {
            const handle = setInterval(() => clearInterval(handle), 1);
            console.log(handle.hasRef());
            handle.unref();
            handle.ref();
        }"#,
    );
}

#[test]
fn discarded_promise_is_tracked_for_unhandled_rejection() {
    let program = lower(
        r#"async function main(): Promise<void> {
            Promise.reject(new Error("boom"));
            await Promise.resolve();
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(matches!(
        &main.body[0],
        HirStmt::Expr(HirExpr::Call(callee, _))
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_detach_rejection")
    ));
}

#[test]
fn lowers_queue_microtask_onto_the_promise_queue() {
    lower(
        r#"async function main(): Promise<void> {
            queueMicrotask(() => console.log("microtask"));
            await Promise.resolve();
        }"#,
    );
}

#[test]
fn typed_quickjs_calls_are_resolved_at_the_dynamic_await_boundary() {
    let program = lower(
        r#"declare function __thaw_typed_js_66(): JsValue;
        async function main(): Promise<void> {
            const value: JsValue = await __thaw_typed_js_66();
            console.log(value);
        }"#,
    );
    assert!(matches!(
        &program.functions[0].body[0],
        HirStmt::Let(
            _,
            HirType::JsValue,
            HirExpr::Call(callee, arguments)
        )
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "resolveDynamicValue")
                && matches!(arguments.as_slice(), [HirExpr::DynamicCall(
                    DynamicSignature { backend: DynamicBackend::QuickJs, .. }, _
                )])
    ));
}

#[test]
fn keeps_unannotated_dynamic_method_promises_raw_until_awaited() {
    let program = lower(
        r#"declare function loader(): JsValue;
        async function main(): Promise<void> {
            const client: JsValue = loader();
            const pending = client.read();
            const value = await pending;
            console.log(value);
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(main.body.iter().any(|statement| matches!(
        statement,
        HirStmt::Let(_, HirType::JsValue, HirExpr::Call(callee, _))
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "callDynamicMethodHandleRaw")
    )));
}

#[test]
fn desugars_for_await_of_promise_array_to_awaited_items() {
    let program = lower(
        r#"async function main(): Promise<void> {
            const values: Promise<number>[] = [
                new Promise<number>((resolve, reject) => resolve(1))
            ];
            for await (const value of values) { console.log(value); }
        }"#,
    );
    let HirStmt::While(_, loop_body) = &program.functions[0].body[3] else {
        panic!("expected indexed while loop");
    };
    assert!(matches!(
        &loop_body[0],
        HirStmt::Let(
            _,
            HirType::F64,
            HirExpr::AwaitPromise(indexed, HirType::F64)
        ) if matches!(indexed.as_ref(), HirExpr::TypedIndex(_, _, HirType::Promise(inner)) if inner.as_ref() == &HirType::F64)
    ));
}

#[test]
fn lowers_async_function_unwrapping_promise_and_await() {
    let program = lower(
        r#"async function fetchStage(): Promise<string> {
            const s: string = process.env.STAGE;
            return s;
        }
        async function main(): Promise<void> {
            const stage: string = await fetchStage();
            console.log(stage);
        }"#,
    );

    let fetch_stage = &program.functions[0];
    assert!(fetch_stage.is_async);
    // The function result stays unwrapped for native code generation;
    // the await node records the value carried by its runtime promise.
    assert_eq!(fetch_stage.ret, HirType::Str);

    let main = &program.functions[1];
    assert!(main.is_async);
    assert_eq!(main.ret, HirType::Void);
    assert_eq!(
        main.body[0],
        HirStmt::Let(
            "stage".into(),
            HirType::Str,
            HirExpr::AwaitPromise(
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("fetchStage".into())),
                    vec![],
                )),
                HirType::Str,
            ),
        )
    );
}

#[test]
fn infers_await_sleep_as_void() {
    let program = lower(
        r#"async function main(): Promise<void> {
            await sleep(1);
        }"#,
    );
    let HirStmt::Expr(HirExpr::Await(inner)) = &program.functions[0].body[0] else {
        panic!("expected await expression");
    };
    assert_eq!(
        FnLowerer::new(
            &HashMap::new(),
            &HashMap::new(),
            &GenericInterfaces::new(),
            &EnumValues::new(),
            &EnumReverseValues::new(),
            HirType::Void,
            None,
        )
        .infer_expr_type(&HirExpr::Await(inner.clone()))
        .unwrap(),
        HirType::Void
    );
}

#[test]
fn lowers_void_promise_continuations() {
    let program = lower(
        r#"async function main(): Promise<void> {
            await new Promise<void>((resolve, reject) => resolve()).then(() => {});
            await new Promise<void>((resolve, reject) => reject("failure")).catch(error => {
                console.log(error);
            });
        }"#,
    );
    assert_eq!(program.functions[0].body.len(), 2);
    for stmt in &program.functions[0].body {
        let HirStmt::Expr(HirExpr::Await(inner)) = stmt else {
            panic!("expected awaited continuation");
        };
        assert!(matches!(
            inner.as_ref(),
            HirExpr::PromiseThen(_, _, HirType::Void, HirType::Void, _, false)
        ));
    }
}

#[test]
fn promise_all_requires_homogeneous_promises() {
    let module = thaw_parser::parse_typescript(
        r#"
        async function value(): Promise<number> {
            await sleep(1);
            return 1;
        }
        async function main(): Promise<void> {
            const values: number[] = await Promise.all([value(), sleep(1)]);
            console.log(values.length);
        }
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("Promise.all element 1 resolves to void"));
}

#[test]
fn promise_combinators_accept_homogeneous_array_spreads() {
    let program = lower(
        r#"async function value(input: number): Promise<number> { return input; }
        function pending(): Promise<number>[] { return [value(2), value(3)]; }
        async function main(): Promise<void> {
            await Promise.all([value(1), ...pending()]);
            await Promise.allSettled([...pending(), value(4)]);
            await Promise.race([value(1), ...pending()]);
            await Promise.any([...pending(), value(4)]);
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(matches!(
        &main.body[0],
        HirStmt::Expr(HirExpr::AwaitPromise(inner, _))
            if matches!(inner.as_ref(), HirExpr::PromiseAllArray(array, _)
                if matches!(array.as_ref(), HirExpr::ArrayConcat(_, _)))
    ));
    assert!(matches!(
        &main.body[1],
        HirStmt::Expr(HirExpr::AwaitPromise(inner, _))
            if matches!(inner.as_ref(), HirExpr::PromiseAllSettledArray(array, _)
                if matches!(array.as_ref(), HirExpr::ArrayConcat(_, _)))
    ));
    assert!(matches!(
        &main.body[2],
        HirStmt::Expr(HirExpr::AwaitPromise(inner, _))
            if matches!(inner.as_ref(), HirExpr::PromiseRaceArray(array, _)
                if matches!(array.as_ref(), HirExpr::ArrayConcat(_, _)))
    ));
    assert!(matches!(
        &main.body[3],
        HirStmt::Expr(HirExpr::AwaitPromise(inner, _))
            if matches!(inner.as_ref(), HirExpr::PromiseAnyArray(array, _)
                if matches!(array.as_ref(), HirExpr::ArrayConcat(_, _)))
    ));
}

#[test]
fn promise_race_rejects_empty_mixed_and_non_promise_inputs() {
    let empty = thaw_parser::parse_typescript(
        "async function main(): Promise<void> { await Promise.race([]); }",
    )
    .unwrap();
    assert!(lower_module(&empty)
        .unwrap_err()
        .contains("requires at least one promise"));

    let mixed = thaw_parser::parse_typescript(
        r#"
        async function numberValue(): Promise<number> { return 1; }
        async function stringValue(): Promise<string> { return "x"; }
        async function main(): Promise<void> {
            await Promise.race([numberValue(), stringValue()]);
        }
        "#,
    )
    .unwrap();
    assert!(lower_module(&mixed)
        .unwrap_err()
        .contains("Promise.race element 1 resolves to Str, expected F64"));

    let plain = thaw_parser::parse_typescript(
        "async function main(): Promise<void> { await Promise.race([1]); }",
    )
    .unwrap();
    assert!(lower_module(&plain)
        .unwrap_err()
        .contains("Promise.race element 0 must be a Promise"));
}

#[test]
fn promise_any_rejects_empty_mixed_and_non_promise_inputs() {
    let empty = thaw_parser::parse_typescript(
        "async function main(): Promise<void> { await Promise.any([]); }",
    )
    .unwrap();
    assert!(lower_module(&empty)
        .unwrap_err()
        .contains("requires at least one promise"));

    let mixed = thaw_parser::parse_typescript(
        r#"
        async function numberValue(): Promise<number> { return 1; }
        async function stringValue(): Promise<string> { return "x"; }
        async function main(): Promise<void> {
            await Promise.any([numberValue(), stringValue()]);
        }
        "#,
    )
    .unwrap();
    assert!(lower_module(&mixed)
        .unwrap_err()
        .contains("Promise.any element 1 resolves to Str, expected F64"));

    let plain = thaw_parser::parse_typescript(
        "async function main(): Promise<void> { await Promise.any([1]); }",
    )
    .unwrap();
    assert!(lower_module(&plain)
        .unwrap_err()
        .contains("Promise.any element 0 must be a Promise"));
}

#[test]
fn promise_all_settled_rejects_mixed_and_non_promise_inputs() {
    let mixed = thaw_parser::parse_typescript(
        r#"
        async function numberValue(): Promise<number> { return 1; }
        async function stringValue(): Promise<string> { return "x"; }
        async function main(): Promise<void> {
            await Promise.allSettled([numberValue(), stringValue()]);
        }
        "#,
    )
    .unwrap();
    assert!(lower_module(&mixed)
        .unwrap_err()
        .contains("Promise.allSettled element 1 resolves to Str, expected F64"));

    let plain = thaw_parser::parse_typescript(
        "async function main(): Promise<void> { await Promise.allSettled([1]); }",
    )
    .unwrap();
    assert!(lower_module(&plain)
        .unwrap_err()
        .contains("Promise.allSettled element 0 must be a Promise"));
}

#[test]
fn untyped_promise_reject_does_not_constrain_combinator_values() {
    let module = thaw_parser::parse_typescript(
        r#"
        async function main(): Promise<void> {
            await Promise.all([Promise.resolve(1), Promise.reject(new Error("all"))]);
            await Promise.allSettled([Promise.reject("left"), Promise.resolve(2)]);
            await Promise.race([Promise.reject("race")]);
            await Promise.any([Promise.reject("any"), Promise.resolve(3)]);
        }
        "#,
    )
    .unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn rejects_async_function_not_declared_as_returning_promise() {
    let module =
        thaw_parser::parse_typescript("async function f(): number { return 1; }").unwrap();
    let err = lower_module(&module).unwrap_err();
    assert!(err.contains("Promise"), "unexpected error: {err}");
}

#[test]
fn lowers_fetch_and_json_parse_field_access() {
    let program = lower(
        r#"function main(): void {
            const text: string = fetch("https://example.com/api");
            const data = JSON.parse(text);
            const name: string = String(data.name);
            const count: number = Number(data.items[0]);
            console.log(name);
            console.log(count);
        }"#,
    );
    let f = &program.functions[0];

    assert_eq!(
        f.body[0],
        HirStmt::Let(
            "text".into(),
            HirType::Str,
            HirExpr::Call(
                Box::new(HirExpr::Var("fetch".into())),
                vec![HirExpr::Lit(HirLit::Str("https://example.com/api".into()))],
            ),
        )
    );
    assert_eq!(
        f.body[1],
        HirStmt::Let(
            "data".into(),
            HirType::Json,
            HirExpr::Call(
                Box::new(HirExpr::Var("JSON.parse".into())),
                vec![HirExpr::Var("text".into())],
            ),
        )
    );
    assert_eq!(
        f.body[2],
        HirStmt::Let(
            "name".into(),
            HirType::Str,
            HirExpr::JsonAsString(Box::new(HirExpr::JsonGet(
                Box::new(HirExpr::Var("data".into())),
                "name".into(),
            ))),
        )
    );
    assert_eq!(
        f.body[3],
        HirStmt::Let(
            "count".into(),
            HirType::F64,
            HirExpr::JsonAsNumber(Box::new(HirExpr::JsonIndex(
                Box::new(HirExpr::JsonGet(
                    Box::new(HirExpr::Var("data".into())),
                    "items".into(),
                )),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            ))),
        )
    );
}
