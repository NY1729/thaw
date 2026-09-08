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
            const empty: { status: string; value: number; reason: string }[] =
                await Promise.allSettled([]);
            console.log(empty.length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_settled_order"),
        "fulfilled\n1\n\nrejected\nbroken\nfulfilled\n3\n3\n0\n"
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
