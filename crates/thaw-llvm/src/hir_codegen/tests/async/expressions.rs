#[test]
fn frame_split_async_functions_return_objects_arrays_and_tuples_from_branches() {
    let source = r#"
        interface Item { value: number; }
        async function objectValue(first: boolean): Promise<Item> {
            await sleep(1);
            if (first) { return { value: 11 }; }
            return { value: 12 };
        }
        async function arrayValue(first: boolean): Promise<number[]> {
            await sleep(1);
            if (first) { return [21, 22]; }
            return [23, 24];
        }
        async function tupleValue(first: boolean): Promise<[number, string]> {
            await sleep(1);
            if (first) { return [31, "first"]; }
            return [32, "second"];
        }
        async function main(): Promise<void> {
            const object: Item = await objectValue(false);
            const array: number[] = await arrayValue(true);
            const tuple: [number, string] = await tupleValue(false);
            console.log(object.value);
            console.log(array[1]);
            console.log(tuple[0]);
            console.log(tuple[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_aggregate_branch_returns"),
        "12\n22\n32\nsecond\n"
    );
}

#[test]
fn frame_split_borrowed_promise_survives_repeated_await_consumers() {
    let source = r#"
        async function main(): Promise<void> {
            const pending: Promise<number> = Promise.resolve(42);
            const first: number = await pending;
            const second: number = await pending;
            console.log(first);
            console.log(second);
        }
    "#;
    assert_eq!(compile_and_run(source, "borrowed_promise_repeated_await"), "42\n42\n");
}

#[test]
fn frame_split_extracts_awaits_from_arguments_and_literals_left_to_right() {
    let source = r#"
        interface Pair { left: number; right: number; }
        async function delayed(value: number, ms: number): Promise<number> {
            await sleep(ms); return value;
        }
        function combine(left: number, right: number): number {
            return left * 10 + right;
        }
        async function main(): Promise<void> {
            const combined: number = combine(
                await delayed(1, 3), await delayed(2, 1)
            );
            const values: number[] = [
                await delayed(3, 2), await delayed(4, 1)
            ];
            const pair: Pair = {
                left: await delayed(5, 2),
                right: await delayed(6, 1)
            };
            console.log(combined);
            console.log(values[0] * 10 + values[1]);
            console.log(pair.left * 10 + pair.right);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "awaits_in_arguments_and_literals"),
        "12\n34\n56\n"
    );
}

#[test]
fn frame_split_extracts_awaits_from_conditional_expressions() {
    let source = r#"
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const first: number = true ? await delayed(42) : 0;
            const second: number = false ? 0 : await delayed(7);
            console.log(first);
            console.log(second);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "awaits_in_conditional_expressions"),
        "42\n7\n"
    );
}

#[test]
fn frame_split_preserves_synchronous_call_arguments_before_await() {
    let source = r#"
        interface Trace { value: string; }
        function record(trace: Trace, label: string, value: number): number {
            trace.value = trace.value + label;
            return value;
        }
        async function delayed(trace: Trace, label: string, value: number): Promise<number> {
            trace.value = trace.value + label;
            await sleep(1);
            return value;
        }
        function combine(first: number, second: number, third: number): number {
            return first * 100 + second * 10 + third;
        }
        function consume(trace: Trace, first: number, second: number): void {
            trace.value = trace.value + String(first) + String(second);
        }
        async function main(): Promise<void> {
            const trace: Trace = { value: "" };
            const value: number = combine(
                record(trace, "a", 1), await delayed(trace, "b", 2), record(trace, "c", 3)
            );
            consume(trace, record(trace, "d", 4), await delayed(trace, "e", 5));
            consume(trace, record(trace, "f", 6), 1 + await delayed(trace, "g", 6));
            console.log(trace.value);
            console.log(value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "call_argument_order_across_await"),
        "abcde45fg67\n123\n"
    );
}

#[test]
fn frame_split_preserves_binary_left_operand_before_nested_await() {
    let source = r#"
        interface Trace { value: string; }
        function record(trace: Trace, label: string, value: number): number {
            trace.value = trace.value + label;
            return value;
        }
        async function delayed(trace: Trace, label: string, value: number): Promise<number> {
            trace.value = trace.value + label;
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const trace: Trace = { value: "" };
            const sum: number = record(trace, "a", 20)
                + await delayed(trace, "b", 22);
            const comparison: boolean = record(trace, "c", 1)
                < await delayed(trace, "d", 2);
            const nested: number = 2 * (record(trace, "e", 3)
                + await delayed(trace, "f", 4));
            console.log(trace.value);
            console.log(sum);
            console.log(comparison);
            console.log(nested);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "binary_order_across_nested_await"),
        "abcdef\n42\ntrue\n14\n"
    );
}

#[test]
fn frame_split_preserves_array_elements_before_nested_await() {
    let source = r#"
        interface Trace { value: string; }
        function record(trace: Trace, label: string, value: number): number {
            trace.value = trace.value + label;
            return value;
        }
        async function delayed(trace: Trace, label: string, value: number): Promise<number> {
            trace.value = trace.value + label;
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const trace: Trace = { value: "" };
            const values: number[] = [
                record(trace, "a", 1),
                await delayed(trace, "b", 2),
                record(trace, "c", 3),
                record(trace, "d", 4) + await delayed(trace, "e", 1)
            ];
            console.log(trace.value);
            console.log(values.join("-"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_order_across_nested_await"),
        "abcde\n1-2-3-5\n"
    );
}

#[test]
fn frame_split_preserves_object_fields_before_nested_await() {
    let source = r#"
        interface Trace { value: string; }
        interface Values { first: number; second: number; third: number; }
        function record(trace: Trace, label: string, value: number): number {
            trace.value = trace.value + label;
            return value;
        }
        async function delayed(trace: Trace, label: string, value: number): Promise<number> {
            trace.value = trace.value + label;
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const trace: Trace = { value: "" };
            const values: Values = {
                first: record(trace, "a", 1),
                second: await delayed(trace, "b", 2),
                third: record(trace, "c", 3) + await delayed(trace, "d", 1)
            };
            console.log(trace.value);
            console.log(values.first + values.second + values.third);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_order_across_nested_await"),
        "abcd\n7\n"
    );
}

#[test]
fn frame_split_preserves_object_methods_and_accessors_across_await() {
    let source = r#"
        interface Trace { value: string; }
        function record(trace: Trace, label: string, value: number): number {
            trace.value += label;
            return value;
        }
        async function delayed(trace: Trace, label: string, value: number): Promise<number> {
            trace.value += label;
            await sleep(1);
            return value;
        }
        async function spread(trace: Trace): Promise<{ extra: number }> {
            return { extra: await delayed(trace, "c", 4) };
        }
        async function main(): Promise<void> {
            const trace: Trace = { value: "" };
            let stored = 3;
            const value = {
                first: record(trace, "a", 1),
                second: await delayed(trace, "b", 2),
                get total(): number { return this.first + this.second + stored; },
                set total(next: number) { stored = next; },
                add(): number { return this.first + this.second; },
                ...(await spread(trace)),
                last: record(trace, "d", 5),
            };
            console.log(trace.value, value.add(), value.total, value.extra, value.last);
            value.total = 10;
            console.log(value.total);

            const overwritten = {
                get result(): number { return 99; },
                result: await delayed(trace, "e", 6),
            };
            const accessor = {
                result: await delayed(trace, "f", 7),
                get result(): number { return 8; },
            };
            console.log(overwritten.result, accessor.result, trace.value);

            const callback = {
                async run(): Promise<number> {
                    return await delayed(trace, "g", 9);
                }
            };
            console.log(await callback.run(), trace.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_members_across_await"),
        "abcd 3 6 4 5\n13\n6 8 abcdef\n9 abcdefg\n"
    );
}

#[test]
fn frame_split_returns_from_deep_async_loop_try_and_block_scopes() {
    let source = r#"
        async function delayed(value: number): Promise<number> {
            await sleep(1); return value;
        }
        async function find(): Promise<number> {
            let index: number = 0;
            while (index < 4) {
                try {
                    const current: number = await delayed(index);
                    if (current === 2) {
                        const answer: number = await delayed(current + 40);
                        return answer;
                    }
                } catch (error) {
                    return 0;
                }
                index = index + 1;
            }
            return 1;
        }
        async function main(): Promise<void> {
            const value: number = await find();
            console.log(value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "deep_async_return_control_flow"),
        "42\n"
    );
}

#[test]
fn frame_split_awaits_promises_stored_in_variables_arguments_and_objects() {
    let source = r#"
        interface Holder { pending: Promise<number>; }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }
        async function consume(pending: Promise<number>): Promise<number> {
            return await pending;
        }
        async function main(): Promise<void> {
            const pending: Promise<number> = delayed(41);
            const holder: Holder = { pending: delayed(42) };
            console.log(await consume(pending));
            console.log(await holder.pending);
        }
    "#;
    assert_eq!(compile_and_run(source, "stored_promise_values"), "41\n42\n");
}

#[test]
fn promise_void_continuations_run_and_settle() {
    let source = r#"
        async function main(): Promise<void> {
            await new Promise<void>((resolve, reject) => resolve())
                .then(() => console.log("then"));
            await new Promise<void>((resolve, reject) => reject("failure"))
                .catch(error => console.log(error));
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_void_continuations"),
        "then\nfailure\ndone\n"
    );
}

#[test]
fn sync_main_drains_ready_native_promise_continuations() {
    let source = r#"
        function main(): void {
            Promise.resolve().then((): void => console.log("done"));
        }
    "#;
    assert_eq!(compile_and_run(source, "sync_main_promise_microtask"), "done\n");
}

#[test]
fn promise_then_transforms_all_native_value_shapes() {
    let source = r#"
        interface Item { value: number; }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const text: string = await new Promise<number>((resolve, reject) => {
                resolve(1);
            }).then(value => "ready");
            const flag: boolean = await new Promise<number>((resolve, reject) => {
                resolve(1);
            }).then(value => true);
            const item: Item = await new Promise<number>((resolve, reject) => {
                resolve(9);
            }).then(value => ({ value: value }));
            const values: number[] = await new Promise<number>((resolve, reject) => {
                resolve(10);
            }).then(value => [value, value + 1]);
            const tuple: [number, string] = await new Promise<number>((resolve, reject) => {
                resolve(12);
            }).then(value => [value, "tuple"]);
            const flattened: number = await new Promise<number>((resolve, reject) => {
                resolve(20);
            }).then(value => delayed(value + 1));
            const recovered: number = await new Promise<number>((resolve, reject) => {
                reject("recover asynchronously");
            }).catch(error => delayed(22));
            console.log(text);
            console.log(flag);
            console.log(item.value);
            console.log(values[1]);
            console.log(tuple[0]);
            console.log(tuple[1]);
            console.log(flattened);
            console.log(recovered);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_chain_shapes"),
        "ready\ntrue\n9\n11\n12\ntuple\n21\n22\n"
    );
}

#[test]
fn promise_finally_preserves_settlement_and_waits_for_promises() {
    let source = r#"
        async function cleanup(): Promise<void> {
            await sleep(1);
            console.log("async cleanup");
        }
        async function main(): Promise<void> {
            const fulfilled: number = await new Promise<number>((resolve, reject) => {
                resolve(41);
            }).finally(() => console.log("fulfilled cleanup"));
            console.log(fulfilled);
            const recovered: number = await new Promise<number>((resolve, reject) => {
                reject("original rejection");
            }).finally(() => {
                console.log("rejected cleanup");
            }).catch(error => {
                console.log(error);
                return 42;
            });
            console.log(recovered);
            const waited: number = await new Promise<number>((resolve, reject) => {
                resolve(43);
            }).finally(() => cleanup());
            console.log(waited);
            const replaced: number = await new Promise<number>((resolve, reject) => {
                resolve(1);
            }).finally(() => {
                throw "finally failure";
            }).catch(error => {
                console.log(error);
                return 44;
            });
            console.log(replaced);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_finally"),
        "fulfilled cleanup\n41\nrejected cleanup\noriginal rejection\n42\nasync cleanup\n43\nfinally failure\n44\n"
    );
}

#[test]
fn directly_awaited_finally_uses_async_frame_rejection_handling() {
    let source = r#"
        async function main(): Promise<void> {
            const value: number = await new Promise<number>((resolve, reject) => {
                resolve(41);
            }).finally(() => {
                console.log("fulfilled cleanup");
            });
            console.log(value);
            try {
                await new Promise<number>((resolve, reject) => {
                    reject("finally rejection");
                }).finally(() => console.log("rejected cleanup"));
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "direct_await_finally"),
        "fulfilled cleanup\n41\nrejected cleanup\nfinally rejection\n"
    );
}

#[test]
fn promise_callbacks_accept_named_functions_and_function_variables() {
    let source = r#"
        function executor(
            resolve: (value: number) => void,
            reject: (error: string) => void
        ): void {
            resolve(10);
        }
        function double(value: number): number { return value * 2; }
        function recover(error: string): number {
            console.log(error);
            return 30;
        }
        function cleanup(): void { console.log("named cleanup"); }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const plusOne: (value: number) => number =
                (value: number): number => value + 1;
            const value: number = await new Promise(executor)
                .then(double)
                .then(plusOne)
                .finally(cleanup);
            console.log(value);
            const recovered: number = await new Promise<number>((resolve, reject) => {
                reject("named recovery");
            }).catch(recover);
            console.log(recovered);
            const assimilated: number = await new Promise((resolve, reject) => {
                resolve(delayed(31));
            });
            console.log(assimilated);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_named_callbacks"),
        "named cleanup\n21\nnamed recovery\n30\n31\n"
    );
}

#[test]
fn frame_split_promise_all_composes_with_if_and_while() {
    let source = r#"
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let value: number = 0;
            if (value === 0) {
                value = (await Promise.all([delayed(1)]))[0];
            }
            while (value < 3) {
                value = (await Promise.all([delayed(value + 1)]))[0];
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "promise_all_control_flow"), "3\n");
}

#[test]
fn frame_split_async_function_preserves_arguments_across_await() {
    let source = r#"
        async function compute(a: number, b: number): Promise<number> {
            await sleep(1);
            return a + b;
        }

        async function main(): Promise<void> {
            const value: number = await compute(20, 22);
            console.log(value);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "async_arguments");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("define internal ptr @compute(double"));
    assert!(ir.contains("frame_a"));
    assert!(ir.contains("frame_b"));
    assert_eq!(compile_and_run(source, "async_arguments"), "42\n");
}

#[test]
fn frame_split_async_lambda_preserves_captured_variable_identity() {
    let source = r#"
        async function main(): Promise<void> {
            let count: number = 0;
            const advance = async (): Promise<number> => {
                await Promise.resolve();
                count += 1;
                return count;
            };
            console.log(await advance(), await advance(), count);
        }
    "#;
    assert_eq!(compile_and_run(source, "async_capture_identity"), "1 2 2\n");
}

#[test]
fn conditional_promise_awaits_preserve_selected_arm_ownership() {
    let source = r#"
        async function main(): Promise<void> {
            const borrowed: Promise<number> = Promise.resolve(40);
            const first: number = await (true ? Promise.resolve(1) : borrowed);
            const second: number = await (false ? Promise.resolve(2) : borrowed);
            const nested: number = await (
                true
                    ? (false ? Promise.resolve(3) : borrowed)
                    : Promise.resolve(4)
            );
            const unionPromise: number = await (true ? Promise.resolve(5) : 6);
            const unionValue: number = await (false ? Promise.resolve(7) : 8);
            const nestedUnion: number = await (
                true
                    ? (false ? Promise.resolve(9) : 10)
                    : Promise.resolve(11)
            );
            console.log(first, second, nested, unionPromise, unionValue, nestedUnion);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "conditional_promise_awaits");
    compiler.compile_program(&program).unwrap();
    assert_eq!(compile_and_run(source, "conditional_promise_awaits"), "1 40 40 5 8 10\n");
}

#[test]
fn named_and_closure_returns_retain_borrowed_promises() {
    let source = r#"
        function named(value: Promise<number>, early: boolean): Promise<number> {
            if (early) { return value; }
            return value;
        }
        async function main(): Promise<void> {
            const original: Promise<number> = Promise.resolve(42);
            const arrow = (value: Promise<number>): Promise<number> => value;
            const block = (value: Promise<number>): Promise<number> => {
                if (true) { return value; }
                return Promise.resolve(0);
            };
            const first: number = await named(original, true);
            const second: number = await arrow(original);
            const third: number = await block(original);
            console.log(first, second, third);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "borrowed_promise_returns");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("borrowed_await_retain"));
    assert_eq!(compile_and_run(source, "borrowed_promise_returns"), "42 42 42\n");
}

#[test]
fn discarded_promise_results_release_only_the_selected_owned_value() {
    let source = r#"
        let calls: number = 0;
        function make(): Promise<number> {
            calls += 1;
            return Promise.resolve(calls);
        }
        async function main(): Promise<void> {
            const borrowed: Promise<number> = Promise.resolve(7);
            make();
            borrowed;
            true ? make() : borrowed;
            console.log(calls, await borrowed);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "discarded_promise_results");
    compiler.compile_program(&program).unwrap();
    assert_eq!(compile_and_run(source, "discarded_promise_results"), "2 7\n");
}
