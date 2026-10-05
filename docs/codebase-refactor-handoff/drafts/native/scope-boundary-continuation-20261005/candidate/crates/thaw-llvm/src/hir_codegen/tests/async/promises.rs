#[test]
fn frame_split_async_main_has_resume_function_and_multiple_states() {
    let source = r#"
        async function main(): Promise<void> {
            console.log("state-0");
            await sleep(1);
            console.log("state-1");
            await sleep(1);
            console.log("state-2");
        }
    "#;

    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "frame_split_ir");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("define internal ptr @thaw_user_main()"));
    assert!(ir.contains("define internal void @thaw_user_main.resume"));
    assert!(ir.contains("state_1"));
    assert!(ir.contains("state_2"));

    assert_eq!(
        compile_and_run(source, "frame_split_multiple_awaits"),
        "state-0\nstate-1\nstate-2\n"
    );
}

#[test]
fn frame_async_allocations_guard_null_before_field_writes() {
    let source = r#"
        async function compute(): Promise<number> {
            await sleep(1);
            return 42;
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "async_frame_allocation_guards");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    let body = ir.lines()
        .skip_while(|line| !(line.starts_with("define ") && line.contains("@compute(")))
        .take_while(|line| *line != "}")
        .collect::<Vec<_>>();
    let is_label = |line: &str| line.split(';').next().unwrap().trim_end().ends_with(':');
    let block = |name: &str| {
        let label = format!("{name}:");
        let start = body.iter().position(|line| line.split(';').next().unwrap().trim_end()
            == label).expect(&ir);
        body.iter().skip(start + 1).take_while(|line| !is_label(line))
            .copied().collect::<Vec<_>>().join("\n")
    };
    let entry = block("entry");
    let frame_failed = block("async_frame_allocation_failed");
    let frame_ready = block("async_frame_allocation_ready");
    let completion_failed = block("async_completion_allocation_failed");
    assert!(entry.contains("async_frame_is_null = icmp eq ptr"), "{ir}");
    assert!(!entry.contains("@thaw_promise_new("), "{ir}");
    assert!(!frame_failed.contains("@thaw_promise_new("), "{ir}");
    assert!(!frame_failed.contains("completion_slot"), "{ir}");
    assert!(frame_ready.contains("@thaw_promise_new("), "{ir}");
    assert!(frame_ready.contains("async_completion_is_null = icmp eq ptr"), "{ir}");
    assert!(!completion_failed.contains("completion_slot"), "{ir}");
    assert!(!completion_failed.contains("waiting_slot"), "{ir}");
}

#[test]
fn frame_split_preserves_and_mutates_locals_across_awaits() {
    let source = r#"
        async function main(): Promise<void> {
            let value: number = 40;
            await sleep(1);
            value = value + 2;
            await sleep(1);
            console.log(value);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "frame_local_slots");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("frame_value"));
    assert_eq!(compile_and_run(source, "frame_local_slots"), "42\n");
}

#[test]
fn frame_splits_non_main_async_function() {
    let source = r#"
        async function compute(): Promise<number> {
            await sleep(1);
            return 42;
        }

        function main(): void {
            console.log("main");
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "general_async_frame");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("define internal ptr @compute()"));
    assert!(ir.contains("define internal void @compute.resume"));
    assert_eq!(compile_and_run(source, "general_async_frame"), "main\n");
}

#[test]
fn frame_split_awaits_user_async_function_and_reads_result() {
    let source = r#"
        async function compute(): Promise<number> {
            await sleep(1);
            return 42;
        }

        async function main(): Promise<void> {
            const value: number = await compute();
            console.log(value);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "await_async_result");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("call ptr @compute()"));
    assert!(ir.contains("__thaw_await_0"));
    assert_eq!(compile_and_run(source, "await_async_result"), "42\n");
}

#[test]
fn frame_split_promise_all_preserves_order_and_supports_empty_arrays() {
    let source = r#"
        async function delayed(value: number, milliseconds: number): Promise<number> {
            await sleep(milliseconds);
            return value;
        }

        async function main(): Promise<void> {
            const values: number[] = await Promise.all([
                delayed(1, 45),
                delayed(2, 5),
                delayed(3, 20)
            ]);
            console.log(values[0]);
            console.log(values[1]);
            console.log(values[2]);
            console.log(values.length);
            const empty: number[] = await Promise.all([]);
            console.log(empty.length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_order"),
        "1\n2\n3\n3\n0\n"
    );
}

#[test]
fn frame_split_promise_all_supports_strings_and_booleans() {
    let source = r#"
        async function word(value: string, milliseconds: number): Promise<string> {
            await sleep(milliseconds);
            return value;
        }

        async function flag(value: boolean, milliseconds: number): Promise<boolean> {
            await sleep(milliseconds);
            return value;
        }

        async function main(): Promise<void> {
            const words: string[] = await Promise.all([
                word("first", 20),
                word("second", 1)
            ]);
            const flags: boolean[] = await Promise.all([
                flag(true, 15),
                flag(false, 1)
            ]);
            console.log(words[0]);
            console.log(words[1]);
            console.log(flags[0]);
            console.log(flags[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_typed_scalars"),
        "first\nsecond\ntrue\nfalse\n"
    );
}

#[test]
fn frame_split_promise_all_supports_aggregate_values() {
    let source = r#"
        interface Item { value: number; }

        async function item(value: number, milliseconds: number): Promise<Item> {
            await sleep(milliseconds);
            return { value: value };
        }

        async function row(value: number, milliseconds: number): Promise<number[]> {
            await sleep(milliseconds);
            return [value, value + 1];
        }

        async function main(): Promise<void> {
            const items: Item[] = await Promise.all([item(7, 15), item(9, 1)]);
            const rows: number[][] = await Promise.all([row(3, 12), row(5, 1)]);
            console.log(items[0].value);
            console.log(items[1].value);
            console.log(rows[0][1]);
            console.log(rows[1][0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_aggregate_values"),
        "7\n9\n4\n5\n"
    );
}

#[test]
fn frame_split_promise_all_accepts_a_promise_array_variable() {
    let source = r#"
        async function delayed(value: number, milliseconds: number): Promise<number> {
            await sleep(milliseconds);
            return value;
        }

        async function main(): Promise<void> {
            const pending: Promise<number>[] = [delayed(4, 20), delayed(6, 1)];
            const values: number[] = await Promise.all(pending);
            console.log(values[0]);
            console.log(values[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_array_variable"),
        "4\n6\n"
    );
}

#[test]
fn promise_combinators_consume_union_arrays_without_erasing_member_types() {
    let source = r#"
        let calls = 0;
        function values(numbers: boolean, reject: boolean): Promise<number>[] | Promise<string>[] {
            calls++;
            if (numbers) {
                return reject
                    ? [Promise.reject<number>("number failure")]
                    : [Promise.resolve(1), Promise.resolve(2)];
            }
            return reject
                ? [Promise.reject<string>("string failure"), Promise.resolve("kept")]
                : [Promise.resolve("a"), Promise.resolve("b")];
        }
        async function main(): Promise<void> {
            console.log((await Promise.all(values(true, false))).join(","));
            console.log((await Promise.all(values(false, false))).join(","));
            console.log(String(await Promise.race(values(true, false))));
            console.log(String(await Promise.race(values(false, false))));
            console.log(String(await Promise.any(values(true, false))));
            console.log(String(await Promise.any(values(false, true))));
            try {
                await Promise.all(values(true, true));
            } catch (error) {
                console.log(error);
            }
            const settled = await Promise.allSettled(values(false, true));
            console.log(settled[0].status, settled[0].reason);
            console.log(settled[1].status, String(settled[1].value));
            console.log(calls);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_union_array_combinators"),
        "1,2\na,b\n1\na\n1\nkept\nnumber failure\nrejected string failure\nfulfilled kept\n8\n"
    );
}

#[test]
fn frame_split_promise_combinators_accept_outer_tuple_spreads() {
    let source = r#"
        async function value(input: number): Promise<number> {
            await sleep(1);
            return input;
        }
        async function text(): Promise<string> { return "ready"; }
        async function main(): Promise<void> {
            const all: number[] = await Promise.all(...[[value(1), value(2)]]);
            console.log(all.join(","));
            console.log(await Promise.race(...[[value(3)]]));
            console.log(await Promise.any(...[[value(4)]]));
            const settled = await Promise.allSettled(...[[value(5)]]);
            console.log(settled[0].status);
            console.log(settled[0].value);
            const mixed: [number, string] = await Promise.all(...[[value(6), text()]]);
            console.log(mixed[0]);
            console.log(mixed[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_combinator_outer_tuple_spreads"),
        "1,2\n3\n4\nfulfilled\n5\n6\nready\n"
    );
}

#[test]
fn frame_split_promise_all_supports_heterogeneous_tuples() {
    let source = r#"
        async function numberValue(): Promise<number> {
            await sleep(15);
            return 12;
        }
        async function stringValue(): Promise<string> {
            await sleep(1);
            return "thaw";
        }
        async function boolValue(): Promise<boolean> {
            await sleep(5);
            return true;
        }

        async function main(): Promise<void> {
            const values: [number, string, boolean] = await Promise.all([
                numberValue(), stringValue(), boolValue()
            ]);
            console.log(values[0]);
            console.log(values[1]);
            console.log(values[2]);
            console.log(values.length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_heterogeneous_tuple"),
        "12\nthaw\ntrue\n3\n"
    );
}

#[test]
fn frame_split_promise_all_tuple_supports_aggregate_members() {
    let source = r#"
        interface Item { value: number; }
        async function item(): Promise<Item> {
            await sleep(8);
            return { value: 21 };
        }
        async function row(): Promise<number[]> {
            await sleep(1);
            return [30, 31];
        }
        async function main(): Promise<void> {
            const values: [Item, number[]] = await Promise.all([item(), row()]);
            console.log(values[0].value);
            console.log(values[1][1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_tuple_aggregates"),
        "21\n31\n"
    );
}

#[test]
fn frame_split_promise_all_rejection_enters_nearest_catch() {
    let source = r#"
        async function succeeds(): Promise<number> {
            await sleep(10);
            return 1;
        }

        async function fails(): Promise<number> {
            await sleep(2);
            throw "joined failure";
        }

        async function main(): Promise<void> {
            try {
                const values: number[] = await Promise.all([succeeds(), fails()]);
                console.log(values[0]);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_rejection"),
        "joined failure\n"
    );
}

#[test]
fn frame_split_promise_all_tuple_rejection_enters_nearest_catch() {
    let source = r#"
        async function succeeds(): Promise<number> {
            await sleep(15);
            return 1;
        }
        async function fails(): Promise<string> {
            await sleep(1);
            throw "tuple failure";
        }
        async function main(): Promise<void> {
            try {
                const values: [number, string] = await Promise.all([succeeds(), fails()]);
                console.log(values[0]);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_tuple_rejection"),
        "tuple failure\n"
    );
}

#[test]
fn frame_split_promise_race_uses_the_first_completion() {
    let source = r#"
        async function delayed(value: number, milliseconds: number): Promise<number> {
            await sleep(milliseconds);
            return value;
        }
        async function main(): Promise<void> {
            const value: number = await Promise.race([
                delayed(1, 30), delayed(2, 2), delayed(3, 15)
            ]);
            console.log(value);
            const pending: Promise<number>[] = [delayed(4, 20), delayed(5, 1)];
            const fromArray: number = await Promise.race(pending);
            console.log(fromArray);
        }
    "#;
    assert_eq!(compile_and_run(source, "promise_race_order"), "2\n5\n");
}

#[test]
fn frame_split_empty_promise_race_remains_pending() {
    let source = r#"
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const literal: number = await Promise.race([
                Promise.race([]), delayed(7)
            ]);
            console.log(literal);
            const empty: Promise<number>[] = [];
            const dynamic: number = await Promise.race([
                Promise.race(empty), delayed(8)
            ]);
            console.log(dynamic);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_race_empty_pending"),
        "7\n8\n"
    );
}

#[test]
fn frame_split_promise_race_supports_native_value_shapes() {
    let source = r#"
        interface Item { value: number; }
        async function word(value: string, ms: number): Promise<string> {
            await sleep(ms); return value;
        }
        async function flag(value: boolean, ms: number): Promise<boolean> {
            await sleep(ms); return value;
        }
        async function item(value: number, ms: number): Promise<Item> {
            await sleep(ms); return { value: value };
        }
        async function row(value: number, ms: number): Promise<number[]> {
            await sleep(ms); return [value, value + 1];
        }
        async function main(): Promise<void> {
            const text: string = await Promise.race([word("slow", 15), word("fast", 1)]);
            const yes: boolean = await Promise.race([flag(false, 15), flag(true, 1)]);
            const object: Item = await Promise.race([item(6, 15), item(7, 1)]);
            const values: number[] = await Promise.race([row(8, 15), row(9, 1)]);
            console.log(text);
            console.log(yes);
            console.log(object.value);
            console.log(values[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_race_native_shapes"),
        "fast\ntrue\n7\n10\n"
    );
}

#[test]
fn frame_split_promise_race_rejection_enters_nearest_catch() {
    let source = r#"
        async function succeeds(): Promise<number> {
            await sleep(20); return 1;
        }
        async function fails(): Promise<number> {
            await sleep(1); throw "race failure";
        }
        async function main(): Promise<void> {
            try {
                const value: number = await Promise.race([succeeds(), fails()]);
                console.log(value);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_race_rejection"),
        "race failure\n"
    );
}

#[test]
fn frame_split_promise_any_ignores_rejections_and_accepts_array_variables() {
    let source = r#"
        async function succeeds(value: number, ms: number): Promise<number> {
            await sleep(ms); return value;
        }
        async function fails(ms: number): Promise<number> {
            await sleep(ms); throw "ignored";
        }
        async function main(): Promise<void> {
            const value: number = await Promise.any([
                fails(1), succeeds(4, 20), succeeds(5, 3)
            ]);
            console.log(value);
            const pending: Promise<number>[] = [fails(1), succeeds(6, 3)];
            const fromArray: number = await Promise.any(pending);
            console.log(fromArray);
        }
    "#;
    assert_eq!(compile_and_run(source, "promise_any_success"), "5\n6\n");
}

#[test]
fn frame_split_empty_promise_any_rejects() {
    let source = r#"
        async function main(): Promise<void> {
            try {
                await Promise.any([]);
            } catch (error) {
                console.log(error);
            }
            const empty: Promise<number>[] = [];
            try {
                await Promise.any(empty);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_any_empty_rejection"),
        "All promises were rejected\nAll promises were rejected\n"
    );
}

#[test]
fn promise_executor_throw_preserves_object_identity() {
    let source = r#"
        interface ThrownValue { value: number; }
        async function main(): Promise<void> {
            try {
                await new Promise<void>(() => {
                    throw { value: 17 };
                });
            } catch (error) {
                const object = error as ThrownValue;
                console.log(object.value);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_executor_object_throw"),
        "17\n"
    );
}

#[test]
fn frame_split_promise_any_supports_native_value_shapes() {
    let source = r#"
        interface Item { value: number; }
        async function word(value: string, ms: number): Promise<string> {
            await sleep(ms); return value;
        }
        async function flag(value: boolean, ms: number): Promise<boolean> {
            await sleep(ms); return value;
        }
        async function item(value: number, ms: number): Promise<Item> {
            await sleep(ms); return { value: value };
        }
        async function row(value: number, ms: number): Promise<number[]> {
            await sleep(ms); return [value, value + 1];
        }
        async function main(): Promise<void> {
            const text: string = await Promise.any([word("slow", 10), word("fast", 1)]);
            const yes: boolean = await Promise.any([flag(false, 10), flag(true, 1)]);
            const object: Item = await Promise.any([item(7, 10), item(8, 1)]);
            const values: number[] = await Promise.any([row(9, 10), row(10, 1)]);
            console.log(text);
            console.log(yes);
            console.log(object.value);
            console.log(values[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_any_native_shapes"),
        "fast\ntrue\n8\n11\n"
    );
}

#[test]
fn frame_split_promise_any_all_rejected_enters_nearest_catch() {
    let source = r#"
        async function fails(message: string, ms: number): Promise<number> {
            await sleep(ms); throw message;
        }
        async function main(): Promise<void> {
            try {
                const value: number = await Promise.any([
                    fails("first", 1), fails("second", 3)
                ]);
                console.log(value);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_any_all_rejected"),
        "All promises were rejected\n"
    );
}

#[test]
fn frame_split_promise_all_settled_preserves_order_and_never_rejects() {
    let source = r#"
        async function succeeds(value: number, ms: number): Promise<number> {
            await sleep(ms); return value;
        }
        async function fails(message: string, ms: number): Promise<number> {
            await sleep(ms); throw message;
        }
        async function main(): Promise<void> {
            const results: { status: string; value: number; reason: string }[] =
                await Promise.allSettled([
                    succeeds(1, 15), fails("broken", 1), succeeds(3, 5)
                ]);
            console.log(results[0].status);
            console.log(results[0].value);
            console.log(results[0].reason);
            console.log(results[1].status);
            console.log(results[1].reason);
            console.log(results[2].status);
            console.log(results[2].value);
            console.log(results.length);
            console.log(results.map((result) => result.status).join(","));
            const empty: { status: string; value: number; reason: string }[] =
                await Promise.allSettled([]);
            console.log(empty.length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_settled_order"),
        "fulfilled\n1\n\nrejected\nbroken\nfulfilled\n3\n3\nfulfilled,rejected,fulfilled\n0\n"
    );
}

#[test]
fn frame_split_promise_all_settled_accepts_array_variables() {
    let source = r#"
        async function value(value: number, ms: number): Promise<number> {
            await sleep(ms); return value;
        }
        async function main(): Promise<void> {
            const pending: Promise<number>[] = [value(4, 10), value(5, 1)];
            const results: { status: string; value: number; reason: string }[] =
                await Promise.allSettled(pending);
            console.log(results[0].value);
            console.log(results[1].value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_settled_array"),
        "4\n5\n"
    );
}

#[test]
fn promise_combinators_accept_array_literal_spreads() {
    let source = r#"
        async function succeeds(value: number, ms: number): Promise<number> {
            await sleep(ms); return value;
        }
        async function fails(message: string, ms: number): Promise<number> {
            await sleep(ms); throw message;
        }
        function allBatch(): Promise<number>[] {
            console.log("all-batch");
            return [succeeds(2, 2), succeeds(3, 1)];
        }
        function failedBatch(): Promise<number>[] {
            console.log("failed-batch");
            return [fails("no", 1)];
        }
        async function main(): Promise<void> {
            const all: number[] = await Promise.all([
                succeeds(1, 1), ...allBatch()
            ]);
            console.log(all[0]); console.log(all[1]); console.log(all[2]);
            const settled: { status: string; value: number; reason: string }[] =
                await Promise.allSettled([...failedBatch(), succeeds(4, 1)]);
            console.log(settled[0].status); console.log(settled[0].reason);
            console.log(settled[1].status); console.log(settled[1].value);
            const raced: number = await Promise.race([
                ...[succeeds(9, 10)], succeeds(5, 1)
            ]);
            console.log(raced);
            const any: number = await Promise.any([
                ...[fails("skip", 1)], succeeds(7, 2)
            ]);
            console.log(any);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_combinator_spreads"),
        "all-batch\n1\n2\n3\nfailed-batch\nrejected\nno\nfulfilled\n4\n5\n7\n"
    );
}

#[test]
fn frame_split_promise_all_settled_supports_native_value_shapes() {
    let source = r#"
        interface Item { value: number; }
        async function word(): Promise<string> { await sleep(1); return "text"; }
        async function flag(): Promise<boolean> { await sleep(1); return true; }
        async function item(): Promise<Item> { await sleep(1); return { value: 8 }; }
        async function row(): Promise<number[]> { await sleep(1); return [9, 10]; }
        async function main(): Promise<void> {
            const words: { status: string; value: string; reason: string }[] =
                await Promise.allSettled([word()]);
            const flags: { status: string; value: boolean; reason: string }[] =
                await Promise.allSettled([flag()]);
            const items: { status: string; value: Item; reason: string }[] =
                await Promise.allSettled([item()]);
            const rows: { status: string; value: number[]; reason: string }[] =
                await Promise.allSettled([row()]);
            console.log(words[0].value);
            console.log(flags[0].value);
            console.log(items[0].value.value);
            console.log(rows[0].value[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_settled_shapes"),
        "text\ntrue\n8\n10\n"
    );
}

#[test]
fn promise_combinators_preserve_union_values_and_discriminants() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        async function number(delay: number): Promise<Result> {
            await sleep(delay);
            return { kind: "number", value: 1 };
        }
        async function text(delay: number): Promise<Result> {
            await sleep(delay);
            return { kind: "text", value: "async" };
        }
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        async function main(): Promise<void> {
            const racing: Promise<Result>[] = [text(10), number(1)];
            const raced = await Promise.race(racing);
            print(raced);
            const any = await Promise.any([text(1), number(10)]);
            print(any);
            const settled = await Promise.allSettled([number(1), text(1)]);
            for (const item of settled) {
                if (item.status === "fulfilled") print(item.value);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_union_combinators"),
        "11\nasync!\n11\nasync!\n"
    );
}

#[test]
fn promise_chains_preserve_union_values_and_discriminants() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        async function value(): Promise<Result> {
            await sleep(1);
            return { kind: "number", value: 1 };
        }
        async function failure(): Promise<Result> {
            await sleep(1);
            throw "failed";
        }
        function transform(item: Result): Result {
            if (item.kind === "number") return { kind: "text", value: "then" };
            return item;
        }
        function recover(reason: string): Result {
            return { kind: "text", value: reason };
        }
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        async function main(): Promise<void> {
            const transformed = await value().then(transform);
            print(transformed);
            const recovered = await failure().catch(recover);
            print(recovered);
            const preserved = await value().finally(() => console.log("finally"));
            print(preserved);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_union_chains"),
        "then!\nfailed!\nfinally\n11\n"
    );
}

#[test]
fn promise_chain_callbacks_accept_tuple_spreads() {
    let source = r#"
        async function value(): Promise<number> { return 20; }
        async function failure(): Promise<number> { throw "failed"; }
        function double(input: number): number { return input * 2; }
        function recover(reason: string): number { return reason === "failed" ? 7 : 0; }
        function cleanup(): void { console.log("cleanup"); }
        async function main(): Promise<void> {
            console.log(await value().then(...[double]));
            console.log(await failure().catch(...[recover]));
            console.log(await value().finally(...[cleanup]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_chain_callback_spreads"),
        "40\n7\ncleanup\n20\n"
    );
}

#[test]
fn promise_constructor_executor_accepts_a_tuple_spread() {
    let source = r#"
        function executor(
            resolve: (value: number) => void,
            reject: (reason: string) => void,
        ): void {
            resolve(23);
        }
        async function main(): Promise<void> {
            const pending = new Promise(...[executor]);
            console.log(await pending);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_constructor_executor_spread"),
        "23\n"
    );
}

#[test]
fn promise_constructor_resolve_accepts_values_and_promises_per_call() {
    let source = r#"
        function named(resolve: (value: number | Promise<number>) => void): void {
            resolve(Promise.resolve(5));
        }
        async function main(): Promise<void> {
            const adopted: number = await new Promise<number>((resolve, reject) => {
                resolve(Promise.resolve(7));
                resolve(99);
                reject("late");
            });
            const plain: number = await new Promise<number>((resolve) => {
                resolve(8);
                resolve(Promise.resolve(100));
                throw "late";
            });
            const rejected: number = await new Promise<number>((resolve, reject) => {
                reject("first");
                resolve(Promise.resolve(200));
            }).catch(reason => reason === "first" ? 4 : 0);
            const escaped: number = await new Promise<number>((resolve) => {
                const done = resolve;
                done(Promise.resolve(9));
            });
            const spreadResolve: number = await new Promise<number>((resolve) => {
                resolve(...[Promise.resolve(13)]);
            });
            const namedValue: number = await new Promise<number>(named);
            const spreadValue: number = await new Promise<number>(...[named]);
            const asyncExecutor: number = await new Promise<number>(async (resolve) => {
                resolve(Promise.resolve(10));
            });
            await new Promise<void>((resolve) => {
                resolve(Promise.resolve());
                resolve();
            });
            console.log(adopted, plain, rejected, escaped, spreadResolve, namedValue, spreadValue, asyncExecutor);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_constructor_mixed_resolver"),
        "7 8 4 9 13 5 5 10\n"
    );
}

#[test]
fn promise_constructor_mixed_resolver_preserves_structured_result_layouts() {
    let source = r#"
        async function main(): Promise<void> {
            const absent: number | undefined = await new Promise<number | undefined>((resolve) => {
                resolve(undefined);
                resolve(Promise.resolve(100));
            });
            const optionalPromise: number | undefined = await new Promise<number | undefined>((resolve) => {
                resolve(Promise.resolve(11));
            });
            const tuple: [number, string] = await new Promise<[number, string]>((resolve) => {
                resolve(Promise.resolve([4, "tuple"] as [number, string]));
            });
            const nullable: number | null = await new Promise<number | null>((resolve) => {
                resolve(null);
            });
            const justNull: null = await new Promise<null>((resolve) => {
                resolve(Promise.resolve(null));
            });
            const plainNull: null = await new Promise<null>((resolve) => {
                resolve(null);
            });
            const justUndefined: undefined = await new Promise<undefined>((resolve) => {
                resolve(undefined);
            });
            const object: { value: number } = await new Promise<{ value: number }>((resolve) => {
                resolve(Promise.resolve({ value: 12 }));
            });
            const widened: number | string = await new Promise<number | string>((resolve) => {
                resolve(Promise.resolve("text"));
            });
            console.log(absent, optionalPromise, tuple[0], tuple[1], nullable, justNull, plainNull, justUndefined, object.value, widened);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_constructor_structured_resolver"),
        "undefined 11 4 tuple null null null undefined 12 text\n"
    );
}

#[test]
fn promise_constructor_absent_resolve_arguments_keep_their_effects() {
    let source = r#"
        function getNull(): null {
            console.log("null effect");
            return null;
        }
        function getUndefined(): undefined {
            console.log("undefined effect");
            return undefined;
        }
        async function main(): Promise<void> {
            await new Promise<void>((resolve) => {
                resolve(console.log("void effect"));
                resolve(Promise.resolve());
            });
            await new Promise<undefined>((resolve) => {
                resolve(getUndefined());
            });
            await new Promise<null>((resolve) => {
                resolve(getNull());
            });
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_absent_resolve_effects"),
        "void effect\nundefined effect\nnull effect\n"
    );
}

#[test]
fn promise_constructor_mixed_void_resolver_resumes_without_a_result_payload() {
    let source = r#"
        async function getVoid(): Promise<void> {
            await sleep(1);
            console.log("awaited effect");
        }
        async function main(): Promise<void> {
            await new Promise<void>(async (resolve) => {
                resolve(await getVoid());
            });
            console.log("settled");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_mixed_void_resume"),
        "awaited effect\nsettled\n"
    );
}

/// `new Promise((resolve) => setTimeout(resolve, ms))` -- the standard
/// zero-dependency "sleep" idiom, ubiquitous in real code (retries,
/// throttling, tests) -- used to fail to compile: the executor's
/// resolve-type inference only recognized a literal `resolve(value)`
/// call anywhere in the executor body, not `resolve` handed off *by
/// reference* to another function (here, `setTimeout`) that invokes it
/// later with no arguments. Fixed by defaulting the inferred type to
/// `void` whenever `resolve` is referenced at all but never directly
/// called with a value -- the overwhelmingly common reason to hand a
/// bare `resolve` to something else (a timer, a one-shot event
/// listener, a "done" callback) is signaling completion, not passing a
/// value. Also confirms the executor still actually waits for the
/// timer (not a no-op) by observing the log order around the `await`.
#[test]
fn promise_executor_infers_void_when_resolve_is_passed_by_reference_to_settimeout() {
    let source = r#"
        function sleep(ms: number): Promise<void> {
            return new Promise((resolve) => setTimeout(resolve, ms));
        }
        async function main(): Promise<void> {
            console.log("before");
            await sleep(5);
            console.log("after");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_executor_resolve_by_reference"),
        "before\nafter\n"
    );
}

/// The identical `setTimeout(resolve, ms)` idiom above works when
/// awaited directly inside `main()` -- but a *bare, unawaited* call to
/// some other async function (real trigger: found auditing a real npm
/// package's own internal "fire and forget" pattern) never resumed past
/// its own `await`, even though the `setTimeout` callback itself ran.
/// `main()`'s own completion is driven by a continuation-aware event-
/// loop variant (`thaw_js_run_until_native_resolved`); any other,
/// unawaited async call's completion is only driven by the generic
/// `thaw_js_run_event_loop`, which never polled the native continuation
/// queue a `setTimeout`-resolved native `Promise<T>` pushes its resume
/// callback onto -- so the resume callback (here, printing "after")
/// simply never ran, and the process exited as soon as `main()`
/// (synchronous, returning immediately) itself completed.
#[test]
fn a_bare_unawaited_async_call_resumes_after_a_settimeout_resolved_promise() {
    let source = r#"
        async function helper(): Promise<void> {
            console.log("before");
            await new Promise<void>((resolve) => {
                setTimeout(() => resolve(), 5);
            });
            console.log("after");
        }
        function main(): void {
            helper();
        }
    "#;
    assert_eq!(
        compile_and_run(source, "bare_unawaited_async_call_settimeout"),
        "before\nafter\n"
    );
}

#[test]
fn promise_with_resolvers_exposes_settlement_functions() {
    let source = r#"
        async function main(): Promise<void> {
            const fulfilled = Promise.withResolvers<number>();
            fulfilled.resolve(42);
            console.log(await fulfilled.promise);

            const rejected = Promise.withResolvers<number>();
            rejected.reject("nope");
            console.log(await rejected.promise.catch(error => {
                console.log(error);
                return 7;
            }));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_with_resolvers"),
        "42\nnope\n7\n"
    );
}

/// Once `resolve(inner)` starts adoption, a later call to the paired reject
/// function or an executor throw must not settle the outer promise first.
#[test]
fn promise_constructor_adoption_claims_both_resolvers_and_executor_throw() {
    let source = r#"
        async function main(): Promise<void> {
            const first = Promise.withResolvers<number>();
            const rejectedLater = new Promise<number>((resolve, reject) => {
                resolve(first.promise);
                reject("late");
            });
            first.resolve(7);
            console.log(await rejectedLater);

            const second = Promise.withResolvers<number>();
            const thrownLater = new Promise<number>((resolve, reject) => {
                resolve(second.promise);
                throw new Error("late throw");
            });
            second.resolve(8);
            console.log(await thrownLater);

            const third = Promise.withResolvers<number>();
            const adoptedRejection = new Promise<number>((resolve, reject) => {
                resolve(third.promise);
                reject("late");
            });
            third.reject("inner");
            console.log(await adoptedRejection.catch(error => {
                console.log(error);
                return 9;
            }));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_constructor_adoption_once"),
        "7\n8\ninner\n9\n"
    );
}

#[test]
fn promise_resolver_first_call_wins_after_executor_returns() {
    let source = r#"
        async function main(): Promise<void> {
            const first = Promise.withResolvers<number>();
            first.resolve(4);
            first.reject("late");
            first.resolve(5);
            console.log(await first.promise);

            const second = Promise.withResolvers<number>();
            second.reject("first");
            second.resolve(6);
            console.log(await second.promise.catch(error => {
                console.log(error);
                return 0;
            }));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_resolver_first_call_wins"),
        "4\nfirst\n0\n"
    );
}

#[test]
fn promise_try_wraps_values_promises_and_throws() {
    let source = r#"
        function fail(): number {
            throw new Error("boom");
        }
        async function main(): Promise<void> {
            console.log(await Promise.try(
                (left: number, right: number) => left + right,
                20,
                22,
            ));
            const args: [number, number] = [19, 23];
            console.log(await Promise.try(
                (left: number, right: number) => left + right,
                ...args,
            ));
            console.log(await Promise.try<number>(() => Promise.resolve(7)));
            try {
                await Promise.try<number>(fail);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_try"),
        "42\n42\n7\nboom\n"
    );
}

#[test]
fn synchronous_main_values_are_not_treated_as_promises() {
    for (name, source, expected) in [
        (
            "sync_main_number_result",
            "function main(): number { console.log('number'); return 42; }",
            "number\n",
        ),
        (
            "sync_main_string_result",
            "function main(): string { console.log('string'); return 'done'; }",
            "string\n",
        ),
        (
            "sync_main_boolean_result",
            "function main(): boolean { console.log('boolean'); return true; }",
            "boolean\n",
        ),
        (
            "sync_main_object_result",
            "function main(): { value: number } { console.log('object'); return { value: 42 }; }",
            "object\n",
        ),
    ] {
        assert_eq!(compile_and_run(source, name), expected, "{name}");
    }
}

#[test]
fn promise_returning_main_values_still_drive_completion() {
    let synchronous = r#"
        function main(): Promise<number> {
            console.log("sync-start");
            return Promise.resolve(42).then((value: number): number => {
                console.log(value);
                return value;
            });
        }
    "#;
    assert_eq!(
        compile_and_run(synchronous, "sync_promise_main_result"),
        "sync-start\n42\n"
    );

    let asynchronous = r#"
        async function main(): Promise<number> {
            console.log("async-start");
            await sleep(1);
            console.log("async-done");
            return 42;
        }
    "#;
    assert_eq!(
        compile_and_run(asynchronous, "async_promise_main_result"),
        "async-start\nasync-done\n"
    );
}

#[test]
fn absent_promise_executor_checks_before_allocation() {
    // Direct HIR supplies a Function-typed null pointer before any class
    // method read starts producing this value. The compiler gate must run
    // after the executor expression but before allocating a Promise.
    let resolved = HirType::F64;
    let resolve = HirType::Function(vec![resolved.clone()], Box::new(HirType::Void));
    let reject = HirType::Function(vec![HirType::Str], Box::new(HirType::Void));
    let executor_type = HirType::Function(vec![resolve, reject], Box::new(HirType::Void));
    let absent = HirExpr::OptionalValue(
        Box::new(HirExpr::OptionalNone(executor_type.clone())),
        executor_type.clone(),
    );
    let program = HirProgram {
        functions: vec![
            HirFunction {
                name: "make_absent_executor".into(),
                params: vec![],
                ret: executor_type,
                is_async: false,
                body: vec![HirStmt::Return(Some(absent))],
            },
            HirFunction {
                name: "main".into(),
                params: vec![],
                ret: HirType::Void,
                is_async: false,
                body: vec![
                    HirStmt::Expr(HirExpr::PromiseNew(
                        Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var("make_absent_executor".into())),
                            vec![],
                        )),
                        resolved,
                        false,
                        false,
                    )),
                    HirStmt::Return(None),
                ],
            },
        ],
        ..HirProgram::default()
    };
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "absent_promise_executor_order");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    let main = ir.lines()
        .skip_while(|line| !(line.starts_with("define ") && line.contains("@main(")))
        .take_while(|line| *line != "}")
        .collect::<Vec<_>>()
        .join("\n");
    let evaluate = main.find("@make_absent_executor(").expect(&ir);
    let check = main.find("callable_is_undefined").expect(&ir);
    let allocate = main.find("@thaw_promise_new(").expect(&ir);
    assert!(evaluate < check && check < allocate, "{ir}");
}

#[test]
fn promise_input_allocation_failure_reaches_catch_and_empty_input_skips_allocator() {
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    extern "C" fn fail_allocation(_bytes: u64, _alignment: u64) -> *mut u8 {
        CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::ptr::null_mut()
    }
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "promise_input_allocation_failure");
    compiler.declare_exception_state();
    let pointer = context.ptr_type(AddressSpace::default());
    let word = context.i64_type();
    let allocator = compiler.module.add_function("thaw_arena_alloc",
        pointer.fn_type(&[word.into(), word.into()], false), None);
    for (name, words) in [("nonempty", 1), ("empty", 0)] {
        let probe = compiler.module.add_function(name, context.i8_type().fn_type(&[], false), None);
        let entry = context.append_basic_block(probe, "entry");
        let caught = context.append_basic_block(probe, "caught");
        compiler.builder.position_at_end(entry);
        compiler.push_catch_target(caught);
        compiler.compile_promise_input_words(words, "input").unwrap();
        compiler.pop_catch_target();
        compiler.builder.build_return(Some(&context.i8_type().const_int(1, false))).unwrap();
        compiler.builder.position_at_end(caught);
        compiler.builder.build_return(Some(&context.i8_type().const_zero())).unwrap();
    }
    compiler.module.verify().unwrap();
    let engine = compiler.module.create_jit_execution_engine(inkwell::OptimizationLevel::None).unwrap();
    engine.add_global_mapping(&allocator, fail_allocation as usize);
    CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
    type Probe = unsafe extern "C" fn() -> u8;
    unsafe {
        let nonempty = engine.get_function::<Probe>("nonempty").unwrap();
        let empty = engine.get_function::<Probe>("empty").unwrap();
        assert_eq!(nonempty.call(), 0);
        assert_eq!(empty.call(), 1);
    }
    assert_eq!(CALLS.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn reject_resolvers_record_their_native_string_before_settlement() {
    // Source-level computed, empty, and embedded-NUL values all enter the
    // same Str-typed reject ABI. The runtime copies bytes only when that
    // exact pointer was marked as trusted native text.
    let source = r#"
        function main(): void {
            const computed: string = "a" + "\0b";
            Promise.reject(computed);
            new Promise<number>((resolve, reject) => reject(computed));
            Promise.reject("");
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "reject_native_text_provenance");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    let reject_bodies = ir.split("define internal void @__thaw_promise_reject_")
        .skip(1)
        .map(|section| section.split("\n}").next().unwrap())
        .collect::<Vec<_>>();
    assert!(reject_bodies.len() >= 3, "{ir}");
    for body in reject_bodies {
        let lines = body.lines().collect::<Vec<_>>();
        let marked = lines.iter().position(|line| line.contains("store ptr %")
            && line.contains("@__thaw_pending_exception_native_text")).expect(&ir);
        let settled = lines.iter().position(|line|
            line.contains("@thaw_promise_reject_typed_with_aggregate")).expect(&ir);
        assert!(marked < settled, "{ir}");
    }
}

#[test]
fn promise_constructor_guards_each_allocation_before_writing_or_invoking() {
    let source = r#"
        function main(): void {
            new Promise<number>((resolve) => resolve(1));
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "promise_constructor_allocation_guards");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    let main = ir.lines()
        .skip_while(|line| !(line.starts_with("define ") && line.contains("@thaw_user_main(")))
        .take_while(|line| *line != "}")
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(main.lines().filter(|line| line.contains("special_closure_allocation_failed")
        && line.contains("= icmp eq ptr")).count(), 2, "{ir}");
    // LLVM prints labels as `name: ; preds = ...`. Strip the comment before
    // recognizing each basic-block boundary.
    let is_label = |line: &str| line.split(';').next().unwrap().trim_end().ends_with(':');
    let block_after = |header: &str| {
        let mut lines = main.lines().skip_while(|line| *line != header);
        assert!(lines.next().is_some(), "{ir}");
        lines.take_while(|line| !is_label(line)).collect::<Vec<_>>().join("\n")
    };
    for failed in ["promise_resolve_closure_failed", "promise_reject_closure_failed"] {
        let label = format!("{failed}:");
        let header = main.lines().find(|line| line.split(';').next().unwrap().trim_end()
            == label.as_str()).expect(&ir);
        let body = block_after(header);
        // `thaw_promise_destroy` returns void, so its LLVM call name is not
        // printed. Check the actual call before the failed path exits.
        assert_eq!(body.matches("@thaw_promise_destroy(").count(), 1, "{ir}");
        assert!(!body.contains("executor_code"), "{ir}");
    }
    let missing = main.lines().filter(|line| line.starts_with("special_closure_missing")
        && is_label(line)).collect::<Vec<_>>();
    assert_eq!(missing.len(), 2, "{ir}");
    for missing in missing {
        let body = block_after(missing);
        assert!(!body.contains("store "), "{ir}");
    }
    assert!(main.find("promise_new_missing").unwrap() < main.find("resolver_state_missing").unwrap(), "{ir}");
    assert!(main.find("resolver_state_missing").unwrap() < main.find("special_closure_allocation_failed").unwrap(), "{ir}");
}

#[test]
fn async_arrow_try_returns_use_the_general_frame_path() {
    let source = r#"
        const choose = async (flag: number): Promise<number> => {
            if (flag === 0) {
                try { return 1; } catch (error) { return 3; }
            }
            if (flag === 1) {
                try { throw new Error("caught"); } catch (error) { return 4; }
            }
            return await Promise.resolve(2);
        };
        async function main(): Promise<void> {
            console.log(await choose(0), await choose(1), await choose(2));
        }
    "#;
    assert_eq!(compile_and_run(source, "async_arrow_try_tail_returns"), "1 4 2\n");
}

#[test]
fn native_promise_scope_boundaries_codegen_regression_control() {
    // Source-only control for generic pending errors, an outer Promise that
    // remains live through a local catch, one-arm capture promotion and its
    // sibling/merge, nested function compilation under a loop/catch, and a
    // finally block which replaces a pending return. This control is authored
    // for later user-run validation and was not executed during this change.
    let source = r#"
        function fail(): void { throw new Error("inner"); }
        function main(): void {
            let outer: Promise<number> = Promise.resolve(1);
            try {
                let inner: Promise<number> = Promise.resolve(2);
                fail();
            } catch (error) {
                const keepOuter = (): Promise<number> => outer;
            }
            outer.then((value: number): void => { console.log(value); });

            let captured: Promise<number> = Promise.resolve(3);
            const choose: boolean = true;
            if (choose) {
                const firstArmOnly = (): Promise<number> => captured;
            } else {
                captured = Promise.resolve(4);
            }
            captured.then((value: number): void => { console.log(value); });

            let looped: Promise<number> = Promise.resolve(5);
            while (choose) {
                try {
                    const loopCapture = (): Promise<number> => looped;
                    break;
                } catch (error) {
                    console.log(error);
                }
            }

            try { return; } finally { throw new Error("finally"); }
        }
    "#;
    let parsed = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&parsed).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "native_promise_scope_boundaries");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("cleanup_before_catch"), "{ir}");
    assert!(ir.contains("release_stack_promise"), "{ir}");
    assert!(ir.contains("arena_promise_slot_error"), "{ir}");
    assert!(ir.contains("whilecond"), "{ir}");

    // A top-level explicit throw exercises module-evaluation catch state,
    // and a throwing module initializer also exercises function-local owner
    // cleanup immediately before the module's own explicit throw.
    let module_source = r#"
        function failModule(): void {
            const moduleOwner: Promise<number> = Promise.resolve(9);
            throw new Error("function");
        }
        failModule();
        throw new Error("module");
        function main(): void {}
    "#;
    let parsed = thaw_parser::parse_typescript(module_source).unwrap();
    let program = thaw_hir::lower_module(&parsed).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "module_scope_boundary");
    compiler.compile_program(&program).unwrap();
    assert!(compiler.print_to_string().contains("__thaw_top_level_init"));

    // Resume-entry reset control for the paired catch/loop context state.
    let async_source = r#"
        async function main(): Promise<void> {
            let owned: Promise<number> = Promise.resolve(7);
            try { await Promise.reject(new Error("reject")); }
            catch (error) { console.log(error); }
            await Promise.resolve(8);
        }
    "#;
    let parsed = thaw_parser::parse_typescript(async_source).unwrap();
    let program = thaw_hir::lower_module(&parsed).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "async_scope_boundary");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("thaw_user_main.resume"), "{ir}");
}

#[test]
fn native_promise_scope_tables_restore_after_codegen_errors() {
    // These direct HIR controls intentionally produce codegen errors after a
    // Promise owner and (for try/while) a boundary have been installed. They
    // check Rust-error restoration without running generated native code.
    for (name, kind) in [("try", 0), ("if", 1), ("while", 2)] {
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, &format!("scope_error_{name}"));
        compiler.declare_exception_state();
        let probe = compiler.module.add_function(
            "probe",
            context.void_type().fn_type(&[], false),
            None,
        );
        let entry = context.append_basic_block(probe, "entry");
        compiler.builder.position_at_end(entry);
        let pointer = context.ptr_type(AddressSpace::default());
        let existing = compiler.builder.build_alloca(pointer, "existing_promise").unwrap();
        compiler.register_stack_promise_slot(existing, "existing").unwrap();
        let slots_before = compiler.stack_promise_slots.clone();

        let invalid_local = HirStmt::Let(
            "temporary".into(),
            HirType::Promise(Box::new(HirType::F64)),
            HirExpr::Var("missing_codegen_binding".into()),
        );
        let result = match kind {
            0 => compiler.compile_block(&[HirStmt::Try(
                vec![invalid_local],
                "error".into(),
                vec![],
                None,
            )]),
            1 => compiler.compile_if(
                &HirExpr::Lit(HirLit::Bool(true)),
                &[invalid_local],
                &[],
            ),
            _ => compiler.compile_while(
                &HirExpr::Lit(HirLit::Bool(true)),
                &[invalid_local],
            ),
        };
        assert!(result.is_err(), "{name} control must reach its codegen error");
        assert!(compiler.catch_stack.is_empty(), "{name} catch context leaked");
        assert!(compiler.loop_scopes.is_empty(), "{name} loop cleanup context leaked");
        assert!(compiler.loop_promotion_scopes.is_empty(), "{name} preheader context leaked");
        assert_eq!(compiler.stack_promise_slots, slots_before, "{name} owner map changed");
    }
}

struct ReactivePromotionProbe<'ctx> {
    preheader: inkwell::basic_block::BasicBlock<'ctx>,
    loop_header: inkwell::basic_block::BasicBlock<'ctx>,
    discovery: inkwell::basic_block::BasicBlock<'ctx>,
    outer_catch: inkwell::basic_block::BasicBlock<'ctx>,
    inner_catch: inkwell::basic_block::BasicBlock<'ctx>,
    owner_slots: [PointerValue<'ctx>; 2],
    body_only_slot: PointerValue<'ctx>,
    body_only_flag: PointerValue<'ctx>,
    promise_type: HirType,
    original_terminator: String,
}

fn build_reactive_promotion_probe<'ctx>(
    compiler: &mut HirCompiler<'ctx>,
    malformed_replace_signature: bool,
) -> ReactivePromotionProbe<'ctx> {
    compiler.declare_exception_state();
    let pointer = compiler.context.ptr_type(AddressSpace::default());
    let i64_type = compiler.context.i64_type();
    let slot_replace_type = if malformed_replace_signature {
        compiler.context.void_type().fn_type(
            &[pointer.into(), pointer.into(), pointer.into()],
            false,
        )
    } else {
        compiler.context.i8_type().fn_type(
            &[pointer.into(), pointer.into(), pointer.into()],
            false,
        )
    };
    compiler.module.add_function(
        "thaw_arena_alloc",
        pointer.fn_type(&[i64_type.into(), i64_type.into()], false),
        Some(inkwell::module::Linkage::External),
    );
    compiler.module.add_function(
        "thaw_promise_arena_slot_replace",
        slot_replace_type,
        Some(inkwell::module::Linkage::External),
    );
    compiler.module.add_function(
        "thaw_promise_destroy",
        compiler.context.void_type().fn_type(&[pointer.into()], false),
        Some(inkwell::module::Linkage::External),
    );

    let probe = compiler.module.add_function(
        "reactive_scope_probe",
        compiler.context.void_type().fn_type(&[], false),
        None,
    );
    let preheader = compiler.context.append_basic_block(probe, "preheader");
    let loop_header = compiler.context.append_basic_block(probe, "loop_header");
    let outer_header = compiler.context.append_basic_block(probe, "outer_loop_header");
    let outer_after = compiler.context.append_basic_block(probe, "outer_loop_after");
    let inner_after = compiler.context.append_basic_block(probe, "inner_loop_after");
    let discovery = compiler.context.append_basic_block(probe, "inner_try_body");
    let outer_catch = compiler.context.append_basic_block(probe, "outer_catch");
    let inner_catch = compiler.context.append_basic_block(probe, "inner_catch");

    compiler.builder.position_at_end(preheader);
    let owner_a = compiler.builder.build_alloca(pointer, "capture_a_stack_slot").unwrap();
    compiler.builder.build_store(owner_a, pointer.const_null()).unwrap();
    compiler.register_stack_promise_slot(owner_a, "capture_a").unwrap();
    let owner_a_flag = compiler.stack_promise_slots[&owner_a];
    let owner_b = compiler.builder.build_alloca(pointer, "capture_b_stack_slot").unwrap();
    compiler.builder.build_store(owner_b, pointer.const_null()).unwrap();
    compiler.register_stack_promise_slot(owner_b, "capture_b").unwrap();
    let owner_b_flag = compiler.stack_promise_slots[&owner_b];
    let mut preheader_slots = std::collections::HashMap::new();
    preheader_slots.insert(owner_a, owner_a_flag);
    preheader_slots.insert(owner_b, owner_b_flag);
    let original_terminator = compiler.builder.build_unconditional_branch(loop_header)
        .unwrap().to_string();

    let promise_type = HirType::Promise(Box::new(HirType::F64));
    compiler.variables.insert(
        "capture_a".into(),
        (owner_a, pointer.into()),
    );
    compiler.variables.insert(
        "capture_b".into(),
        (owner_b, pointer.into()),
    );
    compiler.variable_hir_types.insert("capture_a".into(), promise_type.clone());
    compiler.variable_hir_types.insert("capture_b".into(), promise_type.clone());

    // The outer catch and loop exist at the preheader. The body-only slot is
    // allocated later in the discovery block, so it must not enter the saved
    // preheader owner map or boundary.
    compiler.push_catch_target(outer_catch);
    let outer_catch_scope = compiler.catch_stack[0].clone();
    let mut outer_loop_boundary = std::collections::HashSet::new();
    outer_loop_boundary.insert(owner_a);
    outer_loop_boundary.insert(owner_b);
    let outer_loop = LoopScope {
        continue_target: outer_header,
        break_target: outer_after,
        promise_boundary: outer_loop_boundary,
    };

    compiler.builder.position_at_end(discovery);
    let body_only_slot = compiler.builder.build_alloca(pointer, "body_only_stack_slot").unwrap();
    compiler.builder.build_store(body_only_slot, pointer.const_null()).unwrap();
    compiler.register_stack_promise_slot(body_only_slot, "body_only").unwrap();
    let body_only_flag = compiler.stack_promise_slots[&body_only_slot];
    compiler.push_catch_target(inner_catch);
    let inner_loop = LoopScope {
        continue_target: loop_header,
        break_target: inner_after,
        promise_boundary: outer_loop.promise_boundary.clone(),
    };
    compiler.loop_scopes = vec![outer_loop.clone(), inner_loop];
    let mut captured_names = std::collections::HashSet::new();
    captured_names.insert("capture_a".to_string());
    captured_names.insert("capture_b".to_string());
    compiler.loop_promotion_scopes.push(LoopPromotionScope {
        preheader,
        variables: captured_names,
        catches: CatchContext { scopes: vec![outer_catch_scope] },
        stack_promise_slots: preheader_slots,
        outer_loops: vec![outer_loop],
    });
    compiler.active_async_completion = Some(pointer.const_null());

    ReactivePromotionProbe {
        preheader,
        loop_header,
        discovery,
        outer_catch,
        inner_catch,
        owner_slots: [owner_a, owner_b],
        body_only_slot,
        body_only_flag,
        promise_type,
        original_terminator,
    }
}

fn llvm_blocks_with_prefix(ir: &str, prefix: &str) -> Vec<(String, String)> {
    let lines = ir.lines().collect::<Vec<_>>();
    let is_label = |line: &str| line.split(';').next().unwrap().trim_end().ends_with(':');
    let mut blocks = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let label = line.split(';').next().unwrap().trim_end();
        let Some(name) = label.strip_suffix(':') else { continue };
        if !name.starts_with(prefix) { continue; }
        let body = lines.iter().skip(index + 1).take_while(|line| !is_label(line))
            .copied().collect::<Vec<_>>().join("\n");
        blocks.push((name.to_string(), body));
    }
    blocks
}

fn llvm_function_body(ir: &str, symbol: &str) -> String {
    let lines = ir.lines().collect::<Vec<_>>();
    let start = lines.iter().position(|line|
        line.starts_with("define ") && line.contains(symbol)
    ).unwrap_or_else(|| panic!("missing definition for {symbol}\n{ir}"));
    let end = lines.iter().enumerate().skip(start + 1)
        .find_map(|(index, line)| (*line == "}").then_some(index))
        .unwrap_or_else(|| panic!("unterminated definition for {symbol}\n{ir}"));
    lines[start..=end].join("\n")
}

fn llvm_named_block_body(function_body: &str, name: &str) -> String {
    let lines = function_body.lines().collect::<Vec<_>>();
    let is_label = |line: &str| line.split(';').next().unwrap().trim_end().ends_with(':');
    let label = format!("{name}:");
    let start = lines.iter().position(|line|
        line.split(';').next().unwrap().trim_end() == label
    ).unwrap_or_else(|| panic!("missing block {name}\n{function_body}"));
    lines.iter().skip(start + 1).take_while(|line| !is_label(line))
        .copied().collect::<Vec<_>>().join("\n")
}

fn stores_to_global<'a>(body: &'a str, global: &str) -> Vec<&'a str> {
    let destination = format!(", ptr @{global},");
    body.lines().filter(|line| line.contains("store ") && line.contains(&destination))
        .collect()
}

fn pointer_store_source_operand<'a>(line: &'a str, destination: &str) -> &'a str {
    let store = line.trim().strip_prefix("store ptr ").expect("pointer store instruction");
    store.split_once(&format!(", ptr @{destination},"))
        .expect("exact pointer destination").0
}

type ScopeCatchSnapshot<'ctx> = (inkwell::basic_block::BasicBlock<'ctx>,
    std::collections::HashSet<PointerValue<'ctx>>);
type ScopeLoopSnapshot<'ctx> = (inkwell::basic_block::BasicBlock<'ctx>,
    inkwell::basic_block::BasicBlock<'ctx>, std::collections::HashSet<PointerValue<'ctx>>);
type ScopePromotionSnapshot<'ctx> = (
    inkwell::basic_block::BasicBlock<'ctx>,
    std::collections::HashSet<String>,
    Vec<ScopeCatchSnapshot<'ctx>>,
    std::collections::HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    Vec<ScopeLoopSnapshot<'ctx>>,
);

struct CodegenScopeSnapshot<'ctx> {
    variables: std::collections::HashMap<String, (PointerValue<'ctx>, String)>,
    catch_native_text: std::collections::HashMap<String, (
        PointerValue<'ctx>, PointerValue<'ctx>, PointerValue<'ctx>,
        PointerValue<'ctx>, PointerValue<'ctx>,
    )>,
    variable_hir_types: std::collections::HashMap<String, HirType>,
    arena_variables: std::collections::HashSet<String>,
    stack_promise_slots: std::collections::HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    for_iteration_frame_slots: std::collections::HashMap<String, PointerValue<'ctx>>,
    catch_stack: Vec<ScopeCatchSnapshot<'ctx>>,
    loop_scopes: Vec<ScopeLoopSnapshot<'ctx>>,
    loop_promotion_scopes: Vec<ScopePromotionSnapshot<'ctx>>,
    active_async_completion: Option<PointerValue<'ctx>>,
    insertion_block: Option<inkwell::basic_block::BasicBlock<'ctx>>,
}

fn snapshot_codegen_scope<'ctx>(compiler: &HirCompiler<'ctx>) -> CodegenScopeSnapshot<'ctx> {
    CodegenScopeSnapshot {
        variables: compiler.variables.iter().map(|(name, (cell, ty))|
            (name.clone(), (*cell, ty.print_to_string().to_string()))).collect(),
        catch_native_text: compiler.catch_native_text.clone(),
        variable_hir_types: compiler.variable_hir_types.clone(),
        arena_variables: compiler.arena_variables.clone(),
        stack_promise_slots: compiler.stack_promise_slots.clone(),
        for_iteration_frame_slots: compiler.for_iteration_frame_slots.clone(),
        catch_stack: compiler.catch_stack.iter().map(|scope|
            (scope.target, scope.promise_boundary.clone())).collect(),
        loop_scopes: compiler.loop_scopes.iter().map(|scope|
            (scope.continue_target, scope.break_target, scope.promise_boundary.clone())).collect(),
        loop_promotion_scopes: compiler.loop_promotion_scopes.iter().map(|scope| (
            scope.preheader,
            scope.variables.clone(),
            scope.catches.scopes.iter().map(|catch|
                (catch.target, catch.promise_boundary.clone())).collect(),
            scope.stack_promise_slots.clone(),
            scope.outer_loops.iter().map(|loop_scope|
                (loop_scope.continue_target, loop_scope.break_target,
                    loop_scope.promise_boundary.clone())).collect(),
        )).collect(),
        active_async_completion: compiler.active_async_completion,
        insertion_block: compiler.builder.get_insert_block(),
    }
}

fn assert_codegen_scope_unchanged<'ctx>(
    compiler: &HirCompiler<'ctx>,
    before: &CodegenScopeSnapshot<'ctx>,
) {
    let after = snapshot_codegen_scope(compiler);
    assert_eq!(after.variables, before.variables, "variable cells/types changed");
    assert_eq!(after.catch_native_text, before.catch_native_text);
    assert_eq!(after.variable_hir_types, before.variable_hir_types);
    assert_eq!(after.arena_variables, before.arena_variables);
    assert_eq!(after.stack_promise_slots, before.stack_promise_slots,
        "physical Promise slot-to-flag mapping changed");
    assert_eq!(after.for_iteration_frame_slots, before.for_iteration_frame_slots);
    assert_eq!(after.catch_stack, before.catch_stack);
    assert_eq!(after.loop_scopes, before.loop_scopes);
    assert_eq!(after.loop_promotion_scopes, before.loop_promotion_scopes);
    assert_eq!(after.active_async_completion, before.active_async_completion);
    assert_eq!(after.insertion_block, before.insertion_block);
}

fn seed_nonempty_lambda_scope<'ctx>(
    compiler: &mut HirCompiler<'ctx>,
    function_name: &str,
) -> inkwell::basic_block::BasicBlock<'ctx> {
    compiler.declare_runtime_builtins();
    compiler.declare_exception_state();
    let function = compiler.module.add_function(
        function_name,
        compiler.context.void_type().fn_type(&[], false),
        None,
    );
    let entry = compiler.context.append_basic_block(function, "entry");
    compiler.builder.position_at_end(entry);
    let pointer = compiler.context.ptr_type(AddressSpace::default());
    let owner_slot = compiler.builder.build_alloca(pointer, "seeded_outer_promise_slot").unwrap();
    compiler.builder.build_store(owner_slot, pointer.const_null()).unwrap();
    compiler.register_stack_promise_slot(owner_slot, "seeded_outer_promise").unwrap();
    let promise_type = HirType::Promise(Box::new(HirType::F64));
    compiler.variables.insert("seeded_outer_promise".into(), (owner_slot, pointer.into()));
    compiler.variable_hir_types.insert("seeded_outer_promise".into(), promise_type.clone());
    compiler.push_catch_target(compiler.context.append_basic_block(function, "seeded_catch"));

    let loop_continue = compiler.context.append_basic_block(function, "seeded_loop_continue");
    let loop_break = compiler.context.append_basic_block(function, "seeded_loop_break");
    let promise_boundary: std::collections::HashSet<PointerValue<'ctx>> =
        [owner_slot].into_iter().collect();
    let loop_scope = LoopScope {
        continue_target: loop_continue,
        break_target: loop_break,
        promise_boundary: promise_boundary.clone(),
    };
    compiler.loop_scopes.push(loop_scope.clone());
    compiler.loop_promotion_scopes.push(LoopPromotionScope {
        preheader: entry,
        variables: ["seeded_outer_promise".to_string()].into_iter().collect(),
        catches: CatchContext { scopes: compiler.catch_stack.clone() },
        stack_promise_slots: compiler.stack_promise_slots.clone(),
        outer_loops: vec![loop_scope],
    });

    let catch_slots = (0..5).map(|index|
        compiler.builder.build_alloca(pointer, &format!("seeded_catch_metadata_{index}"))
            .unwrap()).collect::<Vec<_>>();
    compiler.catch_native_text.insert("seeded_catch".into(), (
        catch_slots[0], catch_slots[1], catch_slots[2], catch_slots[3], catch_slots[4],
    ));
    let arena_cell = compiler.builder.build_alloca(pointer, "seeded_arena_cell").unwrap();
    compiler.variables.insert("seeded_arena_capture".into(), (arena_cell, pointer.into()));
    compiler.variable_hir_types.insert("seeded_arena_capture".into(), HirType::JsValue);
    compiler.arena_variables.insert("seeded_arena_capture".into());
    let frame_slot = compiler.builder.build_alloca(pointer, "seeded_for_iteration_frame_slot").unwrap();
    compiler.for_iteration_frame_slots.insert("seeded_for_iteration".into(), frame_slot);
    compiler.active_async_completion = Some(pointer.const_null());
    compiler.builder.position_at_end(entry);
    entry
}

fn assert_promise_owner_flag_guards_release(ir: &str, flag_name: &str) {
    let flag = format!("%{flag_name}");
    let load = ir.lines().find(|line|
        line.contains("= load i1") && line.contains(&format!("ptr {flag}"))
    ).unwrap_or_else(|| panic!("missing runtime owner-flag load for {flag_name}\n{ir}"));
    let loaded_value = load.split_once('=').unwrap().0.trim();
    assert!(ir.lines().any(|line|
        line.contains("br i1") && line.contains(loaded_value)
    ), "owner flag {flag_name} does not guard a conditional release:\n{ir}");
}

#[test]
fn reactive_preheader_promotions_preserve_exact_scope_and_successor_context() {
    // Direct helper control: the closures are discovered reactively while an
    // inner catch/loop is active, after a body-only owner exists. Two captures
    // must split the same preheader in succession while failures still target
    // the outer handler and leave the body-only token in the live body map.
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "reactive_preheader_success");
    let probe = build_reactive_promotion_probe(&mut compiler, false);

    compiler.promote_variable_to_arena_cell_with_mode(
        "capture_a", &probe.promise_type, false,
    ).unwrap();
    let first_ready = compiler.loop_promotion_scopes[0].preheader;
    assert_ne!(first_ready, probe.preheader);
    assert_eq!(first_ready.get_terminator().unwrap().to_string(), probe.original_terminator);
    assert!(compiler.stack_promise_slots.contains_key(&probe.owner_slots[1]));
    assert!(compiler.stack_promise_slots.contains_key(&probe.body_only_slot));
    assert!(!compiler.stack_promise_slots.contains_key(&probe.owner_slots[0]));

    compiler.promote_variable_to_arena_cell_with_mode(
        "capture_b", &probe.promise_type, false,
    ).unwrap();
    let second_ready = compiler.loop_promotion_scopes[0].preheader;
    assert_ne!(second_ready, first_ready);
    assert_eq!(second_ready.get_terminator().unwrap().to_string(), probe.original_terminator);
    assert!(second_ready.get_terminator().unwrap().to_string().contains("loop_header"));
    assert_eq!(compiler.builder.get_insert_block(), Some(probe.discovery));
    assert_eq!(compiler.loop_promotion_scopes[0].stack_promise_slots.len(), 0);
    assert_eq!(compiler.stack_promise_slots.len(), 1);
    assert_eq!(compiler.stack_promise_slots.get(&probe.body_only_slot), Some(&probe.body_only_flag));
    assert_eq!(compiler.catch_stack.len(), 2);
    assert_eq!(compiler.catch_stack[0].target, probe.outer_catch);
    assert_eq!(compiler.catch_stack[1].target, probe.inner_catch);
    assert!(compiler.catch_stack[0].promise_boundary.is_empty());
    assert_eq!(compiler.catch_stack[1].promise_boundary,
        [probe.body_only_slot].into_iter().collect());
    assert_eq!(compiler.loop_scopes.len(), 2);
    assert!(compiler.loop_scopes[0].promise_boundary.is_empty());
    assert!(compiler.loop_scopes[1].promise_boundary.is_empty());
    assert_eq!(compiler.loop_promotion_scopes[0].catches.scopes[0].promise_boundary.len(), 0);
    assert_eq!(compiler.loop_promotion_scopes[0].outer_loops[0].promise_boundary.len(), 0);
    assert!(compiler.arena_variables.contains("capture_a"));
    assert!(compiler.arena_variables.contains("capture_b"));
    assert_eq!(compiler.active_async_completion, Some(compiler.context.ptr_type(AddressSpace::default()).const_null()));

    // Error blocks must use the saved outer catch and omit the body-only slot;
    // normal body cleanup must still release the body-only owner by its flag.
    let body_boundary = compiler.loop_scopes[1].promise_boundary.clone();
    compiler.emit_stack_promise_cleanup_except(&body_boundary).unwrap();
    let ir = compiler.print_to_string();
    let failed_blocks = llvm_blocks_with_prefix(&ir, "arena_promise_slot_error");
    assert_eq!(failed_blocks.len(), 2, "{ir}");
    for (name, body) in failed_blocks {
        assert!(body.contains("br label %outer_catch"), "{name}: {body}\n{ir}");
        assert!(!body.contains("body_only_stack_slot"), "{name}: {body}\n{ir}");
        assert!(!body.contains("br label %inner_catch"), "{name}: {body}\n{ir}");
    }
    assert!(ir.contains("body_only_promise_owned"), "{ir}");
    assert!(ir.contains("@thaw_promise_destroy("), "{ir}");
    assert_promise_owner_flag_guards_release(&ir, "capture_a_promise_owned");
    assert_promise_owner_flag_guards_release(&ir, "capture_b_promise_owned");
    assert_promise_owner_flag_guards_release(&ir, "body_only_promise_owned");
}

#[test]
fn reactive_preheader_codegen_error_restores_nonempty_scope_exactly() {
    // A test-only void arena-slot declaration makes the checked status
    // extraction return a Rust codegen error after the preheader terminator
    // has been detached and the saved outer context installed.
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "reactive_preheader_codegen_error");
    let probe = build_reactive_promotion_probe(&mut compiler, true);
    let slots_before = compiler.stack_promise_slots.clone();
    let catches_before = compiler.catch_stack.clone();
    let loops_before = compiler.loop_scopes.clone();
    let saved_scope = compiler.loop_promotion_scopes[0].clone();

    let result = compiler.promote_variable_to_arena_cell_with_mode(
        "capture_a", &probe.promise_type, false,
    );
    assert!(result.is_err());
    assert_eq!(probe.preheader.get_terminator().unwrap().to_string(), probe.original_terminator);
    assert_eq!(compiler.builder.get_insert_block(), Some(probe.discovery));
    assert_eq!(compiler.stack_promise_slots, slots_before);
    assert_eq!(compiler.catch_stack.len(), catches_before.len());
    for (actual, expected) in compiler.catch_stack.iter().zip(&catches_before) {
        assert_eq!(actual.target, expected.target);
        assert_eq!(actual.promise_boundary, expected.promise_boundary);
    }
    assert_eq!(compiler.loop_scopes.len(), loops_before.len());
    for (actual, expected) in compiler.loop_scopes.iter().zip(&loops_before) {
        assert_eq!(actual.continue_target, expected.continue_target);
        assert_eq!(actual.break_target, expected.break_target);
        assert_eq!(actual.promise_boundary, expected.promise_boundary);
    }
    let restored_scope = &compiler.loop_promotion_scopes[0];
    assert_eq!(restored_scope.preheader, saved_scope.preheader);
    assert_eq!(restored_scope.variables, saved_scope.variables);
    assert_eq!(restored_scope.stack_promise_slots, saved_scope.stack_promise_slots);
    assert_eq!(restored_scope.outer_loops.len(), saved_scope.outer_loops.len());
    assert_eq!(restored_scope.catches.scopes.len(), saved_scope.catches.scopes.len());
    for (actual, expected) in restored_scope.catches.scopes.iter().zip(&saved_scope.catches.scopes) {
        assert_eq!(actual.target, expected.target);
        assert_eq!(actual.promise_boundary, expected.promise_boundary);
    }
    for (actual, expected) in restored_scope.outer_loops.iter().zip(&saved_scope.outer_loops) {
        assert_eq!(actual.continue_target, expected.continue_target);
        assert_eq!(actual.break_target, expected.break_target);
        assert_eq!(actual.promise_boundary, expected.promise_boundary);
    }
    assert!(compiler.variables.get("capture_a").is_some_and(|(slot, _)| *slot == probe.owner_slots[0]));
    assert!(!compiler.arena_variables.contains("capture_a"));
    assert_eq!(compiler.active_async_completion,
        Some(compiler.context.ptr_type(AddressSpace::default()).const_null()));
}

#[test]
fn stack_owner_live_merge_preserves_exact_branch_bindings() {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "stack_owner_live_merge");
    compiler.declare_runtime_builtins();
    let function = compiler.module.add_function(
        "merge_probe",
        context.void_type().fn_type(&[], false),
        None,
    );
    let entry = context.append_basic_block(function, "entry");
    compiler.builder.position_at_end(entry);
    let pointer = context.ptr_type(AddressSpace::default());
    let base = compiler.builder.build_alloca(pointer, "base_slot").unwrap();
    let base_flag = compiler.builder.build_alloca(context.bool_type(), "base_flag").unwrap();
    let then_only = compiler.builder.build_alloca(pointer, "then_slot").unwrap();
    let then_flag = compiler.builder.build_alloca(context.bool_type(), "then_flag").unwrap();
    let else_only = compiler.builder.build_alloca(pointer, "else_slot").unwrap();
    let else_flag = compiler.builder.build_alloca(context.bool_type(), "else_flag").unwrap();
    let mut then_slots = std::collections::HashMap::new();
    then_slots.insert(base, base_flag);
    then_slots.insert(then_only, then_flag);
    let mut else_slots = std::collections::HashMap::new();
    else_slots.insert(base, base_flag);
    else_slots.insert(else_only, else_flag);

    let both_live = HirCompiler::merge_live_stack_promise_slots(true, &then_slots, true, &else_slots);
    assert_eq!(both_live.len(), 3);
    assert_eq!(both_live.get(&base), Some(&base_flag));
    assert_eq!(both_live.get(&then_only), Some(&then_flag));
    assert_eq!(both_live.get(&else_only), Some(&else_flag));
    let then_terminated = HirCompiler::merge_live_stack_promise_slots(false, &then_slots, true, &else_slots);
    assert_eq!(then_terminated.len(), 2);
    assert!(!then_terminated.contains_key(&then_only));
    assert_eq!(then_terminated.get(&else_only), Some(&else_flag));
    let both_terminated = HirCompiler::merge_live_stack_promise_slots(false, &then_slots, false, &else_slots);
    assert!(both_terminated.is_empty());

    // The physical slot->flag bindings survive the live-arm union, and every
    // release remains conditional on the actual runtime owner flag.
    compiler.stack_promise_slots = both_live;
    compiler.emit_stack_promise_cleanup().unwrap();
    let ir = compiler.print_to_string();
    for flag in ["base_flag", "then_flag", "else_flag"] {
        assert_promise_owner_flag_guards_release(&ir, flag);
    }
}

#[test]
fn hir_if_live_arm_keeps_sibling_stack_binding_and_runtime_flag() {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "hir_if_live_stack_owner");
    compiler.declare_runtime_builtins();
    compiler.declare_exception_state();
    let function = compiler.module.add_function(
        "merge_probe",
        context.void_type().fn_type(&[], false),
        None,
    );
    let entry = context.append_basic_block(function, "entry");
    compiler.builder.position_at_end(entry);
    let pointer = context.ptr_type(AddressSpace::default());
    let slot = compiler.builder.build_alloca(pointer, "outer_promise_slot").unwrap();
    let flag = compiler.builder.build_alloca(context.bool_type(), "outer_promise_owned").unwrap();
    compiler.builder.build_store(slot, pointer.const_null()).unwrap();
    compiler.builder.build_store(flag, context.bool_type().const_zero()).unwrap();
    compiler.stack_promise_slots.insert(slot, flag);
    compiler.variables.insert("outer_promise".into(), (slot, pointer.into()));
    compiler.variable_hir_types.insert(
        "outer_promise".into(),
        HirType::Promise(Box::new(HirType::F64)),
    );
    let slots_before = compiler.stack_promise_slots.clone();

    // This real first-arm Let registers a new physical Promise owner before
    // the abrupt exit. The sibling must start from the original outer owner
    // map, not the then-only binding left behind by the terminated arm.
    let terminated = compiler.compile_if(
        &HirExpr::Lit(HirLit::Bool(true)),
        &[
            HirStmt::Let(
                "then_only".into(),
                HirType::Promise(Box::new(HirType::F64)),
                HirExpr::Var("outer_promise".into()),
            ),
            HirStmt::Return(None),
        ],
        &[],
    ).unwrap();
    assert!(!terminated);
    assert_eq!(compiler.stack_promise_slots, slots_before,
        "else arm inherited a Promise slot introduced only in the returned then arm");
    assert_eq!(compiler.stack_promise_slots.get(&slot), Some(&flag));
    assert_eq!(compiler.variables.get("outer_promise").map(|(cell, _)| *cell), Some(slot));
    compiler.emit_stack_promise_cleanup().unwrap();
    let ir = compiler.print_to_string();
    assert_promise_owner_flag_guards_release(&ir, "outer_promise_owned");
    assert_promise_owner_flag_guards_release(&ir, "then_only_promise_owned");
    assert!(ir.contains("outer_promise_slot"), "{ir}");
    assert!(ir.contains("then_only"), "{ir}");
}

#[test]
fn native_promise_exception_descriptor_survives_all_cleanup_handoffs() {
    // Codegen-only source control for a Promise-valued explicit throw, the
    // blocking-await rejection transfer, async completion rejection, and a
    // host callback compiled while an enclosing async scope is active.
    let source = r#"
        declare function __thaw_typed_js_696e766f6b65566f6964(
            callback: () => Promise<void>,
        ): void;
        function syncThrow(reason: Promise<number>): void { throw reason; }
        async function asyncThrow(reason: Promise<number>): Promise<void> {
            throw reason;
        }
        async function main(): Promise<void> {
            const reason: Promise<number> = Promise.resolve(23);
            try { syncThrow(reason); }
            catch (error) {
                const preserved = error as Promise<number>;
                console.log(preserved === reason);
            }
            try { console.log(true && (await Promise.reject<number>(reason))); }
            catch (error) {
                const preserved = error as Promise<number>;
                console.log(preserved === reason);
            }
            try { await asyncThrow(reason); }
            catch (error) {
                const preserved = error as Promise<number>;
                console.log(preserved === reason);
            }
            loadScript("globalThis.invokeVoid = callback => { callback(); };");
            __thaw_typed_js_696e766f6b65566f6964(
                (): Promise<void> => Promise.reject<void>(reason),
            );
        }
    "#;
    let parsed = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&parsed).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "native_reason_cleanup_handoffs");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    let sync_throw = llvm_function_body(&ir, "@syncThrow(");
    assert!(sync_throw.lines().any(|line|
        line.contains("call ptr @thaw_exception_native_provenance_new(")
    ), "sync Promise throw did not emit its native provenance producer:\n{ir}");
    let main_resume = llvm_function_body(&ir, "@thaw_user_main.resume(");
    assert!(main_resume.lines().any(|line|
        line.contains("call ptr @thaw_promise_exception_native(")
    ), "blocking/resume rejection path did not read the Promise native descriptor:\n{ir}");
    // The source async function has no await, so this throw is compiled in
    // its first-segment ramp; the generated resume function only dispatches
    // waiting/state machinery.
    let async_throw_ramp = llvm_function_body(&ir, "@asyncThrow(");
    assert!(async_throw_ramp.lines().any(|line|
        line.contains("call i8 @thaw_promise_reject_typed_with_native_provenance(")
    ), "active async ramp did not forward the pending descriptor:\n{ir}");

    // This fixture lowers through QuickJS. Its native closure reaches the JS
    // callback registrar through the pending-guard wrapper; that wrapper then
    // directly invokes the raw native-value adapter. Follow those exact calls.
    let quickjs_registration = main_resume.lines().find(|line|
        line.contains("call ") && line.contains("@thaw_js_register_native_callback(")
            && line.contains("@__thaw_callback_pending_guard_")
    ).unwrap_or_else(|| panic!("QuickJS native callback registration did not receive the pending-guard wrapper:\n{main_resume}"));
    let registration_args = quickjs_registration
        .split_once("@thaw_js_register_native_callback(").unwrap().1
        .split(')').next().unwrap();
    let registration_args = registration_args.split(',').map(str::trim).collect::<Vec<_>>();
    assert_eq!(registration_args.len(), 7, "{quickjs_registration}");
    let callback_guard_arg = registration_args[0];
    assert!(callback_guard_arg.starts_with("ptr @__thaw_callback_pending_guard_"),
        "registration argument 0 must be the callback guard, separate from the finisher at argument 5:\n{quickjs_registration}");
    let finisher_guard_arg = registration_args[5];
    assert!(finisher_guard_arg.starts_with("ptr @__thaw_callback_pending_guard_"),
        "registration argument 5 must be the separate guarded finisher:\n{quickjs_registration}");
    assert_ne!(callback_guard_arg, finisher_guard_arg,
        "callback and finisher must select distinct pending guards:\n{quickjs_registration}");
    let guard_suffix = callback_guard_arg
        .split("@__thaw_callback_pending_guard_").nth(1).unwrap()
        .split(|character: char| !character.is_ascii_digit()).next().unwrap();
    let guard_name = format!("__thaw_callback_pending_guard_{guard_suffix}");
    let registration = main_resume.lines().position(|line| line == quickjs_registration).unwrap();
    let quickjs_call = main_resume.lines().position(|line|
        line.contains("call ") && line.contains("@thaw_js_call_graph_result(")
    ).unwrap_or_else(|| panic!("QuickJS dynamic call using the callback was not emitted:\n{main_resume}"));
    assert!(registration < quickjs_call,
        "guarded native callback must be registered before the QuickJS call:\n{main_resume}");

    let guard_body = llvm_function_body(&ir, &format!("@{guard_name}("));
    let raw_adapter_call = guard_body.lines().find(|line|
        line.contains("call ptr @__thaw_napi_value_callback_")
    ).unwrap_or_else(|| panic!("pending-guard wrapper did not invoke its raw N-API value adapter:\n{guard_body}"));
    let raw_suffix = raw_adapter_call
        .split("@__thaw_napi_value_callback_").nth(1).unwrap()
        .split(|character: char| !character.is_ascii_digit()).next().unwrap();
    let raw_adapter_name = format!("__thaw_napi_value_callback_{raw_suffix}");
    let raw_adapter_body = llvm_function_body(&ir, &format!("@{raw_adapter_name}("));
    assert!(raw_adapter_body.contains("entry:"), "raw N-API callback body missing:\n{raw_adapter_body}");
}

#[test]
fn nested_codegen_scope_contexts_restore_exact_nonempty_state() {
    // The host-adapter macro remains a narrow direct control for its own
    // isolation. Lambda restoration is covered below through each production
    // compiler method and a real missing-variable error in its inner body.
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "nested_scope_context_restore");
    let probe = build_reactive_promotion_probe(&mut compiler, false);
    let before = snapshot_codegen_scope(&compiler);

    let host_adapter: Result<(), String> = isolated_codegen_scope!(&mut compiler, {
        assert!(compiler.catch_stack.is_empty());
        assert!(compiler.loop_scopes.is_empty());
        assert!(compiler.loop_promotion_scopes.is_empty());
        assert!(compiler.stack_promise_slots.is_empty());
        assert!(compiler.active_async_completion.is_none());
        Err("controlled host adapter codegen error".to_string())
    });
    assert!(host_adapter.is_err());
    assert_codegen_scope_unchanged(&compiler, &before);
    assert_eq!(compiler.builder.get_insert_block(), Some(probe.discovery));
}

#[test]
fn compile_lambda_restores_scope_after_real_inner_body_error() {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "sync_lambda_scope_error");
    seed_nonempty_lambda_scope(&mut compiler, "sync_parent");
    let before = snapshot_codegen_scope(&compiler);
    let missing = HirExpr::Var("missing_codegen_binding".into());

    let error = compiler.compile_lambda(&[], &[], &HirType::F64, &missing)
        .err().expect("compile_lambda must reach its missing Var in the generated body");
    assert_eq!(error, "unknown variable `missing_codegen_binding`");
    let ir = compiler.print_to_string();
    let inner = llvm_function_body(&ir, "@__thaw_lambda_0(");
    assert!(inner.contains("entry:"), "inner lambda body was not emitted:\n{ir}");
    assert_eq!(compiler.module.get_function("__thaw_lambda_0").unwrap().get_basic_blocks().len(), 1,
        "error must come from compiling the emitted inner lambda body");
    assert_codegen_scope_unchanged(&compiler, &before);
}

#[test]
fn compile_async_lambda_restores_scope_after_real_inner_body_error() {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "async_lambda_scope_error");
    seed_nonempty_lambda_scope(&mut compiler, "async_parent");
    let before = snapshot_codegen_scope(&compiler);
    let missing = HirExpr::Var("missing_codegen_binding".into());

    let error = compiler.compile_async_lambda(&[], &[], &HirType::F64, &missing)
        .err().expect("compile_async_lambda must reach its missing Var in the generated body");
    assert_eq!(
        error,
        "async lambda `__thaw_async_lambda_0`: unknown variable `missing_codegen_binding`",
    );
    let ir = compiler.print_to_string();
    let inner = llvm_function_body(&ir, "@__thaw_async_lambda_0(");
    assert!(inner.contains("entry:"), "async ramp body was not emitted:\n{ir}");
    assert!(inner.contains("@thaw_arena_alloc("), "async frame planning/codegen was not reached:\n{ir}");
    assert!(inner.contains("@thaw_promise_new("), "async completion setup was not reached:\n{ir}");
    assert_codegen_scope_unchanged(&compiler, &before);
}

#[test]
fn published_throw_keeps_fresh_native_exception_descriptor() {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "published_native_throw_descriptor");
    compiler.declare_exception_state();
    let function = compiler.module.add_function(
        "probe",
        context.void_type().fn_type(&[], false),
        None,
    );
    let entry = context.append_basic_block(function, "entry");
    compiler.builder.position_at_end(entry);
    let fresh_descriptor = compiler.builder.build_global_string_ptr("fresh descriptor", "descriptor")
        .unwrap().as_pointer_value();
    compiler.builder.build_store(compiler.pending_exception_native().as_pointer_value(), fresh_descriptor)
        .unwrap();
    let published_display = HirExpr::Call(
        Box::new(HirExpr::Var("@@thaw_published_exception_text".into())),
        vec![HirExpr::Lit(HirLit::Str("[exception value]".into()))],
    );

    compiler.compile_throw_text(&published_display).unwrap();
    let ir = compiler.print_to_string();
    let probe_body = llvm_function_body(&ir, "@probe(");
    let descriptor_stores = stores_to_global(&probe_body, "__thaw_pending_exception_native");
    assert_eq!(descriptor_stores.len(), 1, "{ir}");
    let descriptor_operand = pointer_store_source_operand(
        descriptor_stores[0], "__thaw_pending_exception_native",
    );
    assert_eq!(descriptor_operand.matches('@').count(), 1, "{ir}");
    assert!(descriptor_operand.contains("@descriptor"), "{ir}");
    assert!(ir.lines().any(|line|
        line.starts_with("@descriptor =") && line.contains("c\"fresh descriptor\\00\"")
    ), "the saved pointer must identify the fresh descriptor bytes:\n{ir}");
    let text_stores = stores_to_global(&probe_body, "__thaw_pending_exception_native_text");
    assert!(!text_stores.is_empty(), "trusted display text was not published:\n{ir}");
    assert!(!text_stores.last().unwrap().contains("store ptr null"), "{ir}");
}

#[test]
fn text_only_throw_clears_stale_native_exception_descriptor() {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "text_only_stale_native_throw");
    compiler.declare_exception_state();
    let function = compiler.module.add_function(
        "probe",
        context.void_type().fn_type(&[], false),
        None,
    );
    let entry = context.append_basic_block(function, "entry");
    compiler.builder.position_at_end(entry);
    let stale_descriptor = compiler.builder.build_global_string_ptr("stale descriptor", "stale_descriptor")
        .unwrap().as_pointer_value();
    let stale_text = compiler.builder.build_global_string_ptr("stale native text", "stale_native_text")
        .unwrap().as_pointer_value();
    let stale_aggregate = compiler.builder.build_global_string_ptr("stale aggregate", "stale_aggregate")
        .unwrap().as_pointer_value();
    let stale_object = compiler.builder.build_global_string_ptr("stale object", "stale_object")
        .unwrap().as_pointer_value();
    compiler.builder.build_store(compiler.pending_exception_native().as_pointer_value(), stale_descriptor)
        .unwrap();
    compiler.builder.build_store(compiler.pending_exception_native_text().as_pointer_value(), stale_text)
        .unwrap();
    compiler.builder.build_store(compiler.pending_exception_aggregate_errors().as_pointer_value(), stale_aggregate)
        .unwrap();
    compiler.builder.build_store(compiler.pending_exception_object().as_pointer_value(), stale_object)
        .unwrap();
    compiler.builder.build_store(
        compiler.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
        context.i64_type().const_int(99, false),
    ).unwrap();
    compiler.builder.build_store(
        compiler.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL).as_pointer_value(),
        context.f64_type().const_float(8.5),
    ).unwrap();
    compiler.builder.build_store(
        compiler.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL).as_pointer_value(),
        context.i64_type().const_int(77, false),
    ).unwrap();
    compiler.builder.build_store(
        compiler.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL).as_pointer_value(),
        context.bool_type().const_all_ones(),
    ).unwrap();
    let text_only = HirExpr::Call(
        Box::new(HirExpr::Var("@@thaw_trusted_exception_text".into())),
        vec![HirExpr::Lit(HirLit::Str("plain finalizer text".into()))],
    );

    compiler.compile_throw_text(&text_only).unwrap();
    let ir = compiler.print_to_string();
    let probe_body = llvm_function_body(&ir, "@probe(");
    let descriptor_stores = stores_to_global(&probe_body, "__thaw_pending_exception_native");
    assert_eq!(descriptor_stores.len(), 2, "{ir}");
    let stale_descriptor_operand = pointer_store_source_operand(
        descriptor_stores[0], "__thaw_pending_exception_native",
    );
    assert_eq!(stale_descriptor_operand.matches('@').count(), 1, "{ir}");
    assert!(stale_descriptor_operand.contains("@stale_descriptor"), "{ir}");
    assert!(descriptor_stores[1].contains("store ptr null, ptr @__thaw_pending_exception_native,"), "{ir}");
    for global in ["__thaw_pending_exception_object", "__thaw_pending_exception_aggregate_errors"] {
        let stores = stores_to_global(&probe_body, global);
        assert!(stores.first().is_some_and(|line| line.contains("@stale_")), "{global}: {ir}");
        assert!(stores.last().is_some_and(|line| line.contains("store ptr null")), "{global}: {ir}");
    }
    let tag_stores = stores_to_global(&probe_body, "__thaw_pending_exception_value_tag");
    assert!(tag_stores.first().is_some_and(|line| line.contains("store i64 99")), "{ir}");
    assert!(tag_stores.last().is_some_and(|line|
        line.contains("store i64 4, ptr @__thaw_pending_exception_value_tag,")
    ), "text-only throw must become the active String packet:\n{ir}");
    let f64_stores = stores_to_global(&probe_body, "__thaw_pending_exception_f64");
    assert!(f64_stores.first().is_some_and(|line| line.contains("8.5")), "{ir}");
    assert!(f64_stores.last().is_some_and(|line|
        line.contains("store double 0.000000e+00, ptr @__thaw_pending_exception_f64,")
    ), "String packet retained stale f64 payload:\n{ir}");
    let i64_stores = stores_to_global(&probe_body, "__thaw_pending_exception_i64");
    assert!(i64_stores.first().is_some_and(|line| line.contains("store i64 77")), "{ir}");
    assert!(i64_stores.last().is_some_and(|line|
        line.contains("store i64 0, ptr @__thaw_pending_exception_i64,")
    ), "String packet retained stale i64 payload:\n{ir}");
    let bool_stores = stores_to_global(&probe_body, "__thaw_pending_exception_bool");
    assert!(bool_stores.first().is_some_and(|line| line.contains("store i1 true")), "{ir}");
    assert!(bool_stores.last().is_some_and(|line|
        line.contains("ptr @__thaw_pending_exception_bool,") &&
            (line.contains("store i1 false") || line.contains("store i1 0"))
    ), "String packet retained stale boolean payload:\n{ir}");
    let text_stores = stores_to_global(&probe_body, "__thaw_pending_exception_native_text");
    assert!(text_stores.first().is_some_and(|line| line.contains("@stale_native_text")), "{ir}");
    assert!(text_stores.last().is_some_and(|line|
        !line.contains("store ptr null") && !line.contains("@stale_native_text")
    ), "trusted finalizer string was not republished:\n{ir}");
}

#[test]
fn blocking_and_async_exception_handoffs_copy_native_descriptor_before_release() {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "native_promise_reason_handoffs");
    compiler.declare_runtime_builtins();
    compiler.declare_exception_state();
    let function = compiler.module.add_function(
        "probe",
        context.void_type().fn_type(&[], false),
        None,
    );
    let entry = context.append_basic_block(function, "entry");
    compiler.builder.position_at_end(entry);
    let pointer = context.ptr_type(AddressSpace::default());
    let promise = compiler.builder.build_global_string_ptr("rejected Promise", "promise")
        .unwrap().as_pointer_value();
    let error = compiler.builder.build_global_string_ptr("pending reason", "reason")
        .unwrap().as_pointer_value();
    compiler.builder.build_store(compiler.pending_exception().as_pointer_value(), error).unwrap();
    compiler.copy_blocking_promise_exception_metadata(promise).unwrap();
    compiler.builder.build_call(
        compiler.module.get_function("thaw_promise_destroy").unwrap(),
        &[promise.into()], "destroy_after_metadata_copy",
    ).unwrap();

    // Give the shared active-async exception trampoline one tracked stack
    // owner. Its rejection call must attach the descriptor before the pending
    // state is cleared and before the runtime cleanup block can release it.
    let stack_owner = compiler.builder.build_alloca(pointer, "async_stack_owner").unwrap();
    compiler.builder.build_store(stack_owner, pointer.const_null()).unwrap();
    compiler.register_stack_promise_slot(stack_owner, "async_stack_owner").unwrap();
    let slot_flag = compiler.stack_promise_slots[&stack_owner];
    compiler.builder.build_store(slot_flag, context.bool_type().const_int(1, false)).unwrap();
    compiler.builder.build_store(compiler.pending_exception_native().as_pointer_value(), error).unwrap();
    compiler.active_async_completion = Some(promise);
    compiler.branch_on_pending_exception().unwrap();

    let ir = compiler.print_to_string();
    let probe_body = llvm_function_body(&ir, "@probe(");

    // Blocking-await metadata is copied in one call block before its explicit
    // destroy call. Restrict matching to the generated probe's `entry` block
    // so runtime declarations cannot satisfy either side of the ordering.
    let entry_body = llvm_named_block_body(&probe_body, "entry");
    let getter_line = entry_body.lines().find(|line|
        line.contains("call ptr @thaw_promise_exception_native(") && line.contains("@promise")
    ).unwrap_or_else(|| panic!("blocking descriptor getter call missing:\n{entry_body}"));
    let getter = entry_body.lines().position(|line| line == getter_line).unwrap();
    let getter_value = getter_line.split_once('=').unwrap().0.trim();
    let copied_descriptor = format!(
        "store ptr {getter_value}, ptr @__thaw_pending_exception_native,",
    );
    let copy = entry_body.lines().position(|line|
        line.contains(&copied_descriptor)
    ).unwrap_or_else(|| panic!("blocking descriptor was not copied into pending state:\n{entry_body}"));
    let destroy = entry_body.lines().position(|line|
        line.contains("call void @thaw_promise_destroy(") && line.contains("@promise")
    ).unwrap_or_else(|| panic!("explicit blocking Promise destroy call missing:\n{entry_body}"));
    assert!(getter < copy && copy < destroy, "blocking descriptor copy must precede release:\n{entry_body}");

    // Active-async rejection is issued only inside `propagate_exception`;
    // after that call the pending globals are cleared and control reaches a
    // release block. Compare instructions and the actual local CFG edge, not
    // declarations or text from unrelated functions/basic blocks.
    let propagate = llvm_named_block_body(&probe_body, "propagate_exception");
    let reject = propagate.lines().position(|line|
        line.contains("call i8 @thaw_promise_reject_typed_with_native_provenance(")
            && line.contains("%pending_native_exception_descriptor")
    ).unwrap_or_else(|| panic!("active-async provenance reject call missing:\n{propagate}"));
    let pending_clear = propagate.lines().position(|line|
        line.contains("store ptr null, ptr @__thaw_pending_exception,")
    ).unwrap_or_else(|| panic!("pending exception was not cleared after handoff:\n{propagate}"));
    let native_clear = propagate.lines().position(|line|
        line.contains("store ptr null, ptr @__thaw_pending_exception_native,")
    )
        .unwrap_or_else(|| panic!("native descriptor was not cleared after handoff:\n{propagate}"));
    assert!(reject < pending_clear, "reject must own the reason before pending state clears:\n{propagate}");
    assert!(reject < native_clear, "reject must attach the descriptor before it is cleared:\n{propagate}");
    assert!(propagate.contains("label %release_stack_promise"),
        "cleanup release block is not reachable from the rejection path:\n{propagate}");
    let release = llvm_named_block_body(&probe_body, "release_stack_promise");
    assert!(release.lines().any(|line|
        line.contains("call void @thaw_promise_destroy(")
    ), "reachable cleanup release block must contain the Promise destroy call:\n{release}");
}
