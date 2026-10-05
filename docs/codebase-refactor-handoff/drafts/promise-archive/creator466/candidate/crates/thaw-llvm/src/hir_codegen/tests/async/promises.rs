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
        compiler.catch_stack.push(caught);
        compiler.compile_promise_input_words(words, "input").unwrap();
        compiler.catch_stack.pop();
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
fn async_await_direct_and_conditional_creator_tickets_are_explicit_in_ir() {
    let source = r#"
        async function choose(input: Promise<number>, fresh: boolean): Promise<number> {
            return await (fresh
                ? new Promise<number>((resolve) => resolve(7))
                : input);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "await_creator_ticket_ir");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("@thaw_promise_new_with_creator_ticket"), "{ir}");
    assert!(ir.contains("@thaw_promise_new_frame_with_creator_ticket"), "{ir}");
    assert!(ir.contains("waiting_creator_cleanup_reservation"), "{ir}");
    assert!(ir.contains("waiting_creator_cleanup"), "{ir}");
    assert!(ir.contains("conditional_creator_ticket = phi i64"), "{ir}");
    assert!(ir.contains("await_creator_ticket = phi i64"), "{ir}");
    assert!(ir.contains("@thaw_promise_destroy_creator_ticket"), "{ir}");
    assert!(!ir.contains("destroy_waiting = call void @thaw_promise_destroy"), "{ir}");
}

#[test]
fn repeated_awaits_reserve_distinct_waiting_cleanup_packets() {
    let source = r#"
        async function twice(): Promise<number> {
            await new Promise<number>((resolve) => resolve(1));
            await new Promise<number>((resolve) => resolve(2));
            return 3;
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "repeated_await_cleanup_ir");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.matches("await_creator_cleanup_oom").count() >= 2, "{ir}");
    assert!(ir.matches("waiting_creator_cleanup_reservation").count() >= 2, "{ir}");
    assert!(ir.contains("await_cleanup_reservation_failed"), "{ir}");
    assert!(ir.contains("reject_await_cleanup_reservation"), "{ir}");
}

#[test]
fn promise_returning_call_carries_branch_creator_ticket_into_await() {
    let source = r#"
        function produce(input: Promise<number>, fresh: boolean): Promise<number> {
            return fresh ? new Promise<number>((resolve) => resolve(9)) : input;
        }
        async function choose(input: Promise<number>, fresh: boolean): Promise<number> {
            return await produce(input, fresh);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "returned_creator_ticket_ir");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("@produce("), "{ir}");
    assert!(ir.contains("conditional_creator_ticket = phi i64"), "{ir}");
    assert!(ir.contains("call_creator_ticket_out"), "{ir}");
    assert!(ir.contains("called_creator_ticket"), "{ir}");
    assert!(ir.contains("await_creator_ticket = phi i64"), "{ir}");
}

#[test]
fn captured_promise_creator_ticket_crosses_closure_return_boundary() {
    let source = r#"
        function capture(input: Promise<number>, fresh: boolean): () => Promise<number> {
            let current = fresh ? new Promise<number>((resolve) => resolve(11)) : input;
            return () => current;
        }
        async function useCaptured(input: Promise<number>, fresh: boolean): Promise<number> {
            return await capture(input, fresh)();
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "captured_creator_ticket_ir");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("captured_creator_ticket_slot"), "{ir}");
    assert!(ir.contains("captured_creator_ticket_entry"), "{ir}");
    assert!(ir.contains("captured_creator_ticket_cell"), "{ir}");
    assert!(ir.contains("track_promoted_creator_ticket"), "{ir}");
    assert!(ir.contains("capture_creator_registration_oom"), "{ir}");
    assert!(ir.contains("capture_creator_registered_status"), "{ir}");
    assert!(ir.contains("untrack_moved_creator_source"), "{ir}");
    assert!(ir.contains("closure_creator_ticket_out"), "{ir}");
}

#[test]
fn loop_preheader_promise_capture_registration_is_checked_before_source_move() {
    let source = r#"
        function captureLoop(input: Promise<number>, count: number): Promise<number> {
            let current: Promise<number> = input;
            while (count > 0) {
                const read = () => current;
                current = read();
                count--;
            }
            return current;
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "loop_capture_registration_ir");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("capture_creator_registration_oom"), "{ir}");
    assert!(ir.contains("capture_creator_registered"), "{ir}");
    assert!(ir.contains("untrack_moved_creator_source"), "{ir}");
}

#[test]
fn quickjs_main_sweeps_discarded_capture_tickets_before_exit() {
    let source = r#"
        function main(): void {
            let current: Promise<number> = new Promise<number>((resolve) => resolve(3));
            const discarded = () => current;
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "capture_sweep_ir");
    // This control isolates the generated QuickJS-backed terminal path.
    compiler.uses_quickjs = true;
    compiler.uses_quickjs_handles = true;
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("register_deferred_cleanup"), "{ir}");
    assert!(ir.contains("enable_native_object_tracing"), "{ir}");
    assert!(ir.contains("sweep_unreachable_capture_cells"), "{ir}");
    assert!(ir.contains("retire_deferred_capture_creator"), "{ir}");
    assert!(ir.contains("terminal_cleanup_turn"), "{ir}");
    assert!(!ir.contains("published_packet_text_owner"), "{ir}");
}

#[test]
fn native_only_main_reports_exact_deferred_packets_without_js_emitter() {
    let source = r#"
        function main(): void {
            let current: Promise<number> = new Promise<number>((resolve) => resolve(3));
            const discarded = () => current;
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "native_capture_sweep_ir");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("__thaw_native_drain_deferred_json"), "{ir}");
    assert!(ir.contains("report_native_exact_packet"), "{ir}");
    assert!(ir.contains("report_native_oom_exact_packet"), "{ir}");
    assert!(ir.contains("mark_native_cleanup_reservation_failure"), "{ir}");
    // Ownership fields are detached from the still-rooted packet before a
    // NativeStr/Json destructor can reenter the cleanup driver.
    assert!(ir.contains("reporter_alias_is_owner"), "{ir}");
    assert!(ir.contains("reporter_live_alias"), "{ir}");
    assert!(ir.contains("retire_native_packet_json"), "{ir}");
    // A finalizer can publish text ownership before the primary exception.
    // The reserved exact packet must carry that owner past this turn.
    assert!(ir.contains("queue_exact_orphan_native_text"), "{ir}");
    assert!(ir.contains("retire_deferred_capture_creator"), "{ir}");
    assert!(ir.contains("register_deferred_cleanup"), "{ir}");
}

#[test]
fn void_resume_status_zero_queues_subscription_owned_exact_source() {
    let source = r#"
        async function awaitAlias(input: Promise<number>): Promise<number> {
            return await input;
        }
        async function catchAlias(input: Promise<number>): Promise<number> {
            try { return await input; }
            catch (error) { return 1; }
        }
        async function throwAfter(input: Promise<number>): Promise<number> {
            await input;
            throw "boom";
        }
        async function rethrowAfter(input: Promise<number>): Promise<number> {
            try { await input; }
            catch (error) { throw error; }
            return 1;
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "await_exact_terminal_source_ir");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("transfer_exact_rejection_source"), "{ir}");
    assert!(ir.contains("queue_exact_failed_source"), "{ir}");
    assert!(ir.contains("exact_failed_source_queued"), "{ir}");
    assert!(ir.contains("@thaw_promise_queue_delivered_source_failure"), "{ir}");
    assert!(ir.contains("queue_exact_share_failure_cause"), "{ir}");
    assert!(ir.contains("@thaw_promise_take_delivered_cause_packet"), "{ir}");
    assert!(ir.contains("queue_caught_transfer_failure_source"), "{ir}");
    assert!(ir.contains("async_terminal_cleanup_slot"), "{ir}");
    assert!(ir.contains("await_cleanup_rejection_terminal"), "{ir}");
    assert!(ir.contains("queue_frame_terminal_exact_exception"), "{ir}");
    assert!(ir.contains("frame_terminal_rooted_frame_field"), "{ir}");
    assert!(ir.contains("frame_terminal_has_exact_original"), "{ir}");
    assert!(ir.contains("rejected_await_promise_oom"), "{ir}");
    // Each await owns an original/cause pair; consuming it must not spend
    // the frame header's terminal authority for a later state.
    assert!(ir.matches("async_promise_terminal_packet").count() >= 2, "{ir}");
    assert!(ir.contains("async_promise_original_owner_token"), "{ir}");
    assert!(ir.contains("queue_original_failed_await"), "{ir}");
    assert!(ir.contains("release_failed_await_request_base"), "{ir}");
    assert!(ir.contains("defer_await_original_json_owner"), "{ir}");
    assert!(ir.contains("accepted_await_settlement_cause"), "{ir}");
    assert!(ir.contains("queue_explicit_throw_original"), "{ir}");
    assert!(ir.contains("explicit_throw_settlement_failed"), "{ir}");
    assert!(ir.contains("defer_explicit_throw_owner"), "{ir}");
    assert!(ir.contains("throw_catch_owner_transferred"), "{ir}");
}

#[test]
fn quickjs_lambda_registers_post_reset_capture_and_exact_packet_drivers() {
    let source = "function handler(event: string): string { return event; }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "lambda_capture_cleanup_ir");
    compiler.uses_quickjs = true;
    compiler.uses_quickjs_handles = true;
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("register_lambda_exact_cleanup"), "{ir}");
    assert!(ir.contains("register_lambda_capture_cleanup"), "{ir}");
    assert!(ir.contains("__thaw_drain_capture_creators"), "{ir}");
}

#[test]
fn promise_local_assignment_moves_creator_ticket_to_await_frame() {
    let source = r#"
        async function choose(input: Promise<number>, replace: boolean): Promise<number> {
            let current: Promise<number> = new Promise<number>((resolve) => resolve(5));
            if (replace) current = input;
            else current = new Promise<number>((resolve) => resolve(7));
            return await current;
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "promise_local_ticket_ir");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("frame_current_creator_ticket"), "{ir}");
    assert!(ir.contains("async_promise_local_cleanup_slot"), "{ir}");
    assert!(ir.contains("retire_terminal_promise_creator"), "{ir}");
    assert!(ir.contains("terminal_promise_cleanup"), "{ir}");
    assert!(ir.contains("loaded_creator_ticket"), "{ir}");
    assert!(ir.contains("release_replaced_creator_ticket"), "{ir}");
    assert!(ir.contains("reserve_replaced_creator_cleanup"), "{ir}");
    assert!(ir.contains("replaced_creator_cleanup_oom"), "{ir}");
    assert!(ir.contains("async_promise_reentry_packet"), "{ir}");
    assert!(ir.contains("transfer_orphan_json_to_ledger"), "{ir}");
    assert!(ir.contains("move_creator_ticket"), "{ir}");
    assert!(ir.contains("await_creator_ticket = phi i64"), "{ir}");
}

#[test]
fn conditional_promise_alias_selects_ticket_source_cell() {
    let source = r#"
        async function choose(first: Promise<number>, second: Promise<number>, flag: boolean): Promise<number> {
            let left: Promise<number> = first;
            let right: Promise<number> = second;
            return await (flag ? left : right);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "conditional_alias_ticket_ir");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("conditional_creator_source = phi ptr"), "{ir}");
    assert!(ir.contains("creator_ticket_source_present"), "{ir}");
}
