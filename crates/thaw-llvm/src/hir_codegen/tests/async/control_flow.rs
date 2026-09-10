#[test]
fn frame_split_supports_await_inside_expressions() {
    let source = r#"
        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            const value: number = (await compute(20)) + (await compute(21)) + 1;
            console.log(value);
            console.log((await compute(6)) * 7);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "await_in_expression");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("frame___thaw_await_0"));
    assert!(ir.contains("frame___thaw_await_1"));
    assert_eq!(compile_and_run(source, "await_in_expression"), "42\n42\n");
}

#[test]
fn frame_split_supports_await_in_if_condition() {
    let source = r#"
        async function isReady(value: number): Promise<boolean> {
            await sleep(1);
            return value > 0;
        }

        async function main(): Promise<void> {
            if (await isReady(1)) {
                console.log("ready");
            } else {
                console.log("not-ready");
            }
            if (await isReady(0)) {
                console.log("unexpected");
            } else {
                console.log("false-branch");
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "await_if_condition"),
        "ready\nfalse-branch\n"
    );
}

#[test]
fn frame_split_supports_await_inside_if_branches() {
    let source = r#"
        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let value: number = 0;
            if (true) {
                value = await compute(20);
            } else {
                console.log("wrong-else");
                value = await compute(100);
            }
            if (false) {
                console.log("wrong-then");
                await sleep(1);
            } else {
                value = value + (await compute(22));
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "await_if_branches"), "42\n");
}

#[test]
fn frame_split_supports_await_inside_while_loop() {
    let source = r#"
        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let index: number = 0;
            let total: number = 0;
            while (index < 3) {
                await sleep(1);
                total = total + (await compute(index));
                index = index + 1;
            }
            console.log(total);
            console.log(index);
        }
    "#;
    assert_eq!(compile_and_run(source, "await_while_loop"), "3\n3\n");
}

/// Baseline coverage for a local declared *inside* an async `while`
/// loop's body (re-declared every iteration, unlike a local declared
/// before the loop) and captured by the closure `&&`/`||` desugar into
/// (`lower_logical_expr`/`wrap_call_argument_bindings`). This exact
/// shape, but with the awaited value coming from a real npm package's
/// dynamic/QuickJS-backed call (Hono's `app.request(...)`/`response.
/// text()`) rather than a plain recursive `async` helper as here,
/// segfaulted or silently misevaluated on the loop's second and later
/// iterations -- see `registry_add_runs_a_real_hono_route_repeatedly_
/// through_a_while_loop_when_enabled` in `thaw-cli`'s `applications.rs`
/// for the actual reproducing regression test and root-cause writeup;
/// this plain-`sleep`-based version alone was confirmed to pass
/// identically with or without that fix, so it's kept only as ordinary
/// baseline coverage for the pattern, not as a reproduction of the bug
/// itself.
///
/// Fixed (see the real-Hono test for the confirmed root cause) by
/// having `HirStmt::Let` reuse an existing async-frame cell (just
/// `build_store` into it) instead of always allocating a fresh one,
/// whenever the name being declared is already bound to one.
#[test]
fn frame_split_supports_compound_condition_over_a_string_declared_inside_an_async_while_loop() {
    let source = r#"
        async function fetchName(n: number): Promise<string> {
            await sleep(1);
            return "ok" + n;
        }

        async function main(): Promise<void> {
            let count: number = 0;
            let stable: boolean = true;
            while (stable && count < 5) {
                const name: string = await fetchName(count);
                if (name !== "ok" + count) {
                    stable = false;
                } else {
                    count = count + 1;
                }
            }
            console.log(count);
            console.log(stable);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "compound_condition_string_in_while"),
        "5\ntrue\n"
    );
}

#[test]
fn frame_split_supports_await_and_continue_in_do_while_loop() {
    let source = r#"
        async function main(): Promise<void> {
            let i = 0;
            do {
                await sleep(1);
                i++;
                if (i === 1) continue;
                console.log(i);
            } while (i < 3);
            console.log("done");
        }
    "#;
    assert_eq!(compile_and_run(source, "await_do_while"), "2\n3\ndone\n");
}

#[test]
fn frame_split_supports_await_in_switch_tests_and_cases() {
    let source = r#"
        async function selected(): Promise<string> {
            await sleep(1);
            console.log("selected");
            return "b";
        }
        async function main(): Promise<void> {
            let result = 0;
            switch ("b") {
                case "a": console.log("wrong-a"); break;
                case await selected():
                    console.log("matched");
                    await sleep(1);
                    result = 2;
                default:
                    console.log("fallthrough-default");
                    result = result + 1;
                    break;
                case "never": console.log("wrong-never");
            }
            console.log(result);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "await_switch"),
        "selected\nmatched\nfallthrough-default\n3\n"
    );
}

#[test]
fn frame_split_supports_await_in_string_for_of_loop() {
    let source = r#"
        async function main(): Promise<void> {
            const values: string[] = ["first", "skip", "last"];
            for (const value of values) {
                await sleep(1);
                if (value === "skip") continue;
                console.log(value);
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "await_for_of"),
        "first\nlast\ndone\n"
    );
}

#[test]
fn frame_split_supports_for_await_of_and_rejection_catch() {
    let source = r#"
        async function main(): Promise<void> {
            const values: Promise<number>[] = [
                new Promise<number>((resolve, reject) => resolve(1)),
                new Promise<number>((resolve, reject) => resolve(2)),
                new Promise<number>((resolve, reject) => resolve(3))
            ];
            for await (const value of values) {
                if (value === 2) continue;
                console.log(value);
            }
            let last = 0;
            const assigned: Promise<number>[] = [
                new Promise<number>((resolve, reject) => resolve(4)),
                new Promise<number>((resolve, reject) => resolve(5))
            ];
            for await (last of assigned) {}
            console.log(last);
            for await (const immediate of [6, 7]) {
                console.log(immediate);
            }
            try {
                const failures: Promise<number>[] = [
                    new Promise<number>((resolve, reject) => reject("for await failure"))
                ];
                for await (const value of failures) {
                    console.log(value);
                }
            } catch (error) {
                console.log(error);
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "for_await_of"),
        "1\n3\n5\n6\n7\nfor await failure\ndone\n"
    );
}

#[test]
fn frame_split_supports_await_in_while_condition() {
    let source = r#"
        async function shouldContinue(value: number): Promise<boolean> {
            await sleep(1);
            return value < 3;
        }

        async function main(): Promise<void> {
            let index: number = 0;
            while (await shouldContinue(index)) {
                console.log(index);
                index = index + 1;
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "await_while_condition"),
        "0\n1\n2\ndone\n"
    );
}

#[test]
fn frame_split_supports_deeply_nested_if_awaits() {
    let source = r#"
        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let value: number = 0;
            if (true) {
                if (false) {
                    console.log("wrong-inner-then");
                    value = 100;
                } else {
                    if (true) {
                        value = await compute(42);
                    }
                }
            } else {
                console.log("wrong-outer-else");
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "nested_async_if"), "42\n");
}

#[test]
fn frame_split_supports_await_in_condition_and_branch_body() {
    let source = r#"
        async function ready(value: boolean): Promise<boolean> {
            await sleep(1);
            return value;
        }

        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let value: number = 0;
            if (await ready(true)) {
                value = await compute(42);
            } else {
                value = await compute(100);
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "async_if_condition_body"), "42\n");
}

#[test]
fn frame_split_supports_await_in_deeply_nested_if_condition() {
    let source = r#"
        async function ready(value: boolean): Promise<boolean> {
            await sleep(1);
            return value;
        }

        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let value: number = 0;
            if (true) {
                if (await ready(false)) {
                    value = await compute(100);
                } else {
                    if (await ready(true)) {
                        value = await compute(42);
                    }
                }
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "nested_async_if_condition"), "42\n");
}

#[test]
fn frame_split_supports_nested_async_while_loops() {
    let source = r#"
        async function below(value: number, limit: number): Promise<boolean> {
            await sleep(1);
            return value < limit;
        }

        async function main(): Promise<void> {
            let outer: number = 0;
            let inner: number = 0;
            let total: number = 0;
            while (outer < 2) {
                await sleep(1);
                inner = 0;
                while (await below(inner, 3)) {
                    total = total + 1;
                    inner = inner + 1;
                }
                outer = outer + 1;
            }
            console.log(total);
            console.log(outer);
        }
    "#;
    assert_eq!(compile_and_run(source, "nested_async_while"), "6\n2\n");
}

#[test]
fn frame_split_supports_async_try_finally_normal_completion() {
    let source = r#"
        async function main(): Promise<void> {
            try {
                console.log("try");
                await sleep(1);
                console.log("resumed");
            } finally {
                console.log("finally");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_try_finally_normal"),
        "try\nresumed\nfinally\ndone\n"
    );
}

#[test]
fn frame_split_runs_finally_before_async_return() {
    let source = r#"
        async function main(): Promise<void> {
            try {
                await sleep(1);
                console.log("returning");
                return;
            } finally {
                console.log("finally");
            }
            console.log("unreachable");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_try_finally_return"),
        "returning\nfinally\n"
    );
}

#[test]
fn frame_split_propagates_async_rejection_through_await_chain() {
    let source = r#"
        async function fail(): Promise<number> {
            await sleep(1);
            throw "boom";
        }

        async function relay(): Promise<number> {
            const value: number = await fail();
            return value;
        }

        async function main(): Promise<void> {
            console.log("before");
            await relay();
            console.log("unreachable");
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "async_rejection_ir");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("call i8 @thaw_promise_reject"));
    assert!(ir.contains("propagate_rejection"));
    assert_eq!(compile_and_run(source, "async_rejection_chain"), "before\n");
}

#[test]
fn frame_split_catches_throw_after_await_in_same_function() {
    let source = r#"
        async function main(): Promise<void> {
            try {
                console.log("try");
                await sleep(1);
                throw "boom";
                console.log("unreachable");
            } catch (error) {
                console.log(error);
                await sleep(1);
                console.log("caught");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_direct_catch"),
        "try\nboom\ncaught\ndone\n"
    );
}

#[test]
fn awaits_promise_returned_by_a_synchronous_function() {
    let source = r#"
        function value(): Promise<number> {
            return new Promise<number>((resolve) => resolve(42));
        }
        async function main(): Promise<void> {
            console.log(await value());
        }
    "#;
    assert_eq!(compile_and_run(source, "sync_promise_return"), "42\n");
}

#[test]
fn frame_split_catches_rejected_child_promise() {
    let source = r#"
        async function fail(): Promise<void> {
            await sleep(1);
            throw "child-boom";
        }

        async function main(): Promise<void> {
            try {
                console.log("before");
                await fail();
                console.log("unreachable");
            } catch (error) {
                console.log(error);
                await sleep(1);
                console.log("caught");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_rejection_catch"),
        "before\nchild-boom\ncaught\ndone\n"
    );
}

#[test]
fn frame_split_rethrows_after_await_in_catch() {
    let source = r#"
        async function fail(): Promise<void> {
            await sleep(1);
            throw "original";
        }

        async function relay(): Promise<void> {
            try {
                await fail();
            } catch (error) {
                console.log(error);
                await sleep(1);
                console.log("rethrowing");
                throw "wrapped";
            }
            console.log("relay-unreachable");
        }

        async function main(): Promise<void> {
            console.log("before");
            await relay();
            console.log("main-unreachable");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_catch_rethrow"),
        "before\noriginal\nrethrowing\n"
    );
}

#[test]
fn frame_split_selects_nearest_catch_in_nested_async_try() {
    let source = r#"
        async function failInner(): Promise<void> {
            await sleep(1);
            throw "inner-error";
        }

        async function failOuter(): Promise<void> {
            await sleep(1);
            throw "outer-error";
        }

        async function main(): Promise<void> {
            try {
                try {
                    await failInner();
                } catch (inner) {
                    console.log(inner);
                    await failOuter();
                    console.log("inner-unreachable");
                }
                console.log("outer-try-unreachable");
            } catch (outer) {
                console.log(outer);
                await sleep(1);
                console.log("outer-caught");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nested_async_try_catch"),
        "inner-error\nouter-error\nouter-caught\ndone\n"
    );
}

#[test]
fn frame_split_supports_arbitrarily_deep_async_try_catch() {
    let source = r#"
        async function failDeep(): Promise<void> {
            await sleep(1);
            throw "deep";
        }

        async function main(): Promise<void> {
            try {
                try {
                    try {
                        await failDeep();
                    } catch (deep) {
                        console.log(deep);
                        await sleep(1);
                        throw "middle";
                    }
                } catch (middle) {
                    console.log(middle);
                    await sleep(1);
                    throw "outer";
                }
            } catch (outer) {
                console.log(outer);
                await sleep(1);
                console.log("caught");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "deep_async_try_catch"),
        "deep\nmiddle\nouter\ncaught\ndone\n"
    );
}

#[test]
fn frame_split_supports_locals_and_return_inside_async_branch() {
    let source = r#"
        async function value(): Promise<number> {
            await sleep(1);
            return 42;
        }

        async function choose(): Promise<number> {
            if (1 < 2) {
                const answer = await value();
                return answer;
            }
            return 0;
        }

        async function main(): Promise<void> {
            const result = await choose();
            console.log(result);
        }
    "#;
    assert_eq!(compile_and_run(source, "async_branch_local_return"), "42\n");
}

#[test]
fn frame_split_supports_try_inside_async_if_and_while() {
    let source = r#"
        async function main(): Promise<void> {
            let i = 0;
            if (1 < 2) {
                try {
                    const label = "if-error";
                    await sleep(1);
                    throw label;
                } catch (error) {
                    console.log(error);
                }
            }
            while (i < 1) {
                try {
                    const label = "while-error";
                    await sleep(1);
                    throw label;
                } catch (error) {
                    console.log(error);
                }
                i = i + 1;
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_try_in_if_while"),
        "if-error\nwhile-error\ndone\n"
    );
}

#[test]
fn frame_split_supports_async_if_and_while_inside_try() {
    let source = r#"
        async function main(): Promise<void> {
            let i = 0;
            try {
                if (1 < 2) {
                    const prefix = "branch";
                    await sleep(1);
                    console.log(prefix);
                }
                while (i < 2) {
                    const current = i;
                    await sleep(1);
                    i = current + 1;
                    if (i > 1) {
                        throw "loop-error";
                    }
                }
                console.log("unreachable");
            } catch (error) {
                console.log(error);
                await sleep(1);
                console.log("caught");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_if_while_in_try"),
        "branch\nloop-error\ncaught\ndone\n"
    );
}
