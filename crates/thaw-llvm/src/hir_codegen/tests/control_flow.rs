#[test]
fn compiles_inferred_returns_and_locals_to_native_layouts() {
    let source = r#"
        interface Box<T> {
            value: T;
        }
        interface Wrapper<T> {
            boxed: Box<T>;
        }
        interface Pair<T, U> {
            first: T;
            second: U;
        }

        function first() {
            return second();
        }

        function second() {
            const base = 40;
            return identity(base) + 2;
        }

        function identity<T>(value: T): T {
            return value;
        }

        function sameArray<T>(value: T[]): T[] {
            return value;
        }

        function sameBox<T>(value: Box<T>): Box<T> {
            return value;
        }

        function sameWrapper<T>(value: Wrapper<T>): Wrapper<T> {
            return value;
        }

        function samePair<T, U>(value: Pair<T, U>): Pair<T, U> {
            return value;
        }

        function firstOf<T, U>(value: Pair<T, U>): T {
            return value.first;
        }

        function unbox<T>(value: Wrapper<T>): T {
            return value.boxed.value;
        }

        function main(): void {
            const answer = first();
            console.log(answer);
            console.log(identity("ok"));
            const values = sameArray([7, 8]);
            console.log(values[1]);
            const boxed = sameBox({ value: 9 });
            console.log(boxed.value);
            const wrapped = sameWrapper({ boxed: { value: 10 } });
            console.log(wrapped.boxed.value);
            const pair = samePair({ first: 11, second: 12 });
            console.log(pair.first);
            console.log(pair.second);
            console.log(firstOf({ first: 13, second: 14 }));
            console.log(unbox({ boxed: { value: 15 } }));
        }
    "#;

    assert_eq!(
        compile_and_run(source, "inferred_types"),
        "42\nok\n8\n9\n10\n11\n12\n13\n15\n"
    );
}

#[test]
fn compiles_if_else_and_recursion() {
    let source = r#"
        function fib(n: number): number {
            if (n < 2) {
                return n;
            } else {
                return fib(n - 1) + fib(n - 2);
            }
        }

        function main(): void {
            console.log(fib(10));
        }
    "#;
    assert_eq!(compile_and_run(source, "fib"), "55\n");
}

#[test]
fn compiles_do_while_with_continue_and_break() {
    let source = r#"
        function main(): void {
            let i = 0;
            do {
                i++;
                if (i === 1) continue;
                console.log(i);
                if (i === 3) break;
            } while (i < 5);
        }
    "#;
    assert_eq!(compile_and_run(source, "do_while"), "2\n3\n");
}

#[test]
fn compiles_switch_selection_default_fallthrough_and_break() {
    let source = r#"
        function probe(label: string, value: number): number {
            console.log(label);
            return value;
        }
        function main(): void {
            switch (2) {
                case probe("first", 1): console.log("wrong"); break;
                default: console.log("default-before");
                case probe("second", 2):
                    console.log("two");
                case 3:
                    console.log("three");
                    break;
                case probe("never", 4): console.log("unreachable");
            }
            switch ("missing") {
                case "x": console.log("x"); break;
                default: console.log("fallback");
                case "later": console.log("after-default");
            }
            switch (1) {
                case 1:
                    let i = 0;
                    while (i < 1) { i++; break; }
                    console.log(i);
                    break;
                default: console.log("wrong-default");
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "switch_flow"),
        "first\nsecond\ntwo\nthree\nfallback\nafter-default\n1\n"
    );
}

#[test]
fn compiles_for_of_with_single_source_evaluation_continue_and_break() {
    let source = r#"
        function values(): number[] {
            console.log("values");
            return [1, 2, 3, 4];
        }
        function main(): void {
            for (const value of values()) {
                if (value === 2) continue;
                console.log(value);
                if (value === 3) break;
            }
        }
    "#;
    assert_eq!(compile_and_run(source, "for_of"), "values\n1\n3\n");
}

#[test]
fn compiles_for_of_assignment_and_retains_last_value() {
    let source = r#"
        function main(): void {
            let value = 0;
            for (value of [4, 5, 6]) {
                console.log(value);
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "for_of_assignment"), "4\n5\n6\n6\n");
}

#[test]
fn compiles_try_catch_within_a_single_function() {
    let source = r#"
        function main(): void {
            try {
                console.log("before");
                throw "boom";
                console.log("unreachable");
            } catch (e) {
                console.log(e);
            }
            console.log("after");
        }
    "#;
    assert_eq!(compile_and_run(source, "trycatch"), "before\nboom\nafter\n");
}

#[test]
fn compiles_throw_new_error_constructors() {
    let source = r#"
        function main(): void {
            try {
                throw new Error("boom");
            } catch (e) {
                console.log(e);
            }
            try {
                throw new TypeError("wrong type");
            } catch (e) {
                console.log(e);
            }
            try {
                throw new Error();
            } catch (e) {
                console.log("[" + e.message + "]");
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "throw_new_error"),
        "boom\nwrong type\n[]\n"
    );
}

#[test]
fn caught_errors_expose_message_name_and_instanceof() {
    let source = r#"
        function main(): void {
            try {
                throw new TypeError("wrong type");
            } catch (e) {
                console.log(e.message);
                console.log(e.name);
                console.log(e instanceof TypeError);
                console.log(e instanceof Error);
                console.log(e instanceof RangeError);
            }
            try {
                throw "plain string";
            } catch (e) {
                console.log(e.message);
                console.log(e.name);
                console.log(e instanceof Error);
                console.log(e instanceof TypeError);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "caught_error_properties"),
        "wrong type\nTypeError\ntrue\ntrue\nfalse\nplain string\nError\ntrue\nfalse\n"
    );
}

#[test]
fn propagates_throw_across_function_calls() {
    let source = r#"
        function deepest(): string {
            throw "cross-function boom";
        }

        function middle(): string {
            return deepest();
        }

        function main(): void {
            try {
                console.log(middle());
                console.log("unreachable");
            } catch (e) {
                console.log(e);
            }
            console.log("after");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "cross_function_trycatch"),
        "cross-function boom\nafter\n"
    );
}

#[test]
fn a_catch_can_rethrow_to_an_outer_function() {
    let source = r#"
        function inner(): string {
            try {
                throw "first";
            } catch (e) {
                console.log(e);
                throw "second";
            }
        }

        function main(): void {
            try {
                console.log(inner());
            } catch (e) {
                console.log(e);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "cross_function_rethrow"),
        "first\nsecond\n"
    );
}

#[test]
fn finally_runs_on_normal_return_and_caught_throw() {
    let source = r#"
        function returns(): string {
            try {
                return "value";
            } finally {
                console.log("finally-return");
            }
        }

        function catches(): string {
            try {
                throw "boom";
            } catch (e) {
                console.log(e);
                return "caught";
            } finally {
                console.log("finally-catch");
            }
        }

        function main(): void {
            console.log(returns());
            console.log(catches());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "finally_return_and_catch"),
        "finally-return\nvalue\nboom\nfinally-catch\ncaught\n"
    );
}

#[test]
fn try_finally_without_catch_rethrows_after_cleanup() {
    let source = r#"
        function fails(): string {
            try {
                throw "boom";
            } finally {
                console.log("cleanup");
            }
        }

        function main(): void {
            try {
                console.log(fails());
            } catch (e) {
                console.log(e);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "finally_rethrow"),
        "cleanup\nboom\n"
    );
}

#[test]
fn a_throw_from_finally_bypasses_the_same_try_catch() {
    let source = r#"
        function fails(): string {
            try {
                console.log("try");
            } catch (e) {
                console.log("wrong-catch");
            } finally {
                throw "from-finally";
            }
            return "unreachable";
        }

        function main(): void {
            try {
                console.log(fails());
            } catch (e) {
                console.log(e);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "finally_throw_bypasses_catch"),
        "try\nfrom-finally\n"
    );
}

#[test]
fn return_from_finally_overrides_an_earlier_return() {
    let source = r#"
        function value(): string {
            try {
                return "try";
            } finally {
                return "finally";
            }
        }

        function main(): void {
            console.log(value());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "finally_return_override"),
        "finally\n"
    );
}

#[test]
fn quickjs_errors_use_thaw_try_catch_and_finally() {
    let source = r#"
        function main(): void {
            loadScript(
                "function boom() { throw new Error('kaboom'); } function later() { return Promise.reject(new Error('nope')); } function badThen() { return Object.create(null, { then: { get() { throw new Error('bad then'); } } }); }"
            );
            try {
                const ignored = callDynamic("boom", JSON.parse("[]"));
                console.log("unreachable");
            } catch (error) {
                console.log(error);
            } finally {
                console.log("cleanup");
            }
            try {
                const ignored = callDynamic("later", JSON.parse("[]"));
            } catch (error) {
                console.log(error);
            }
            try {
                const ignored = callDynamic("badThen", JSON.parse("[]"));
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "quickjs_result_abi"),
        "`boom` threw: kaboom\ncleanup\n`later`'s promise rejected: nope\n`badThen`'s promise rejected: bad then\n"
    );
}

#[test]
fn guards_top_level_initialization_against_reentry() {
    let source = r#"
        const answer = 42;
        function main(): void { console.log(answer); }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "guarded_top_level_init");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.module.print_to_string().to_string();
    assert!(ir.contains("@__thaw_top_level_initialized = internal global i1 false"));
    assert!(ir.contains("define internal void @__thaw_top_level_init()"));
    assert!(ir.contains("br i1 %top_level_initialized"));
}

#[test]
fn frame_split_void_return_resolves_completion_and_stops() {
    let source = r#"
        async function main(): Promise<void> {
            console.log("before");
            await sleep(1);
            console.log("resumed");
            return;
            console.log("unreachable");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "frame_void_return"),
        "before\nresumed\n"
    );
}

#[test]
fn frame_split_value_return_is_stored_in_completion_result_slot() {
    let source = r#"
        async function main(): Promise<number> {
            console.log("before");
            await sleep(1);
            return 42;
            console.log("unreachable");
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "frame_value_return");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("async_result"));
    assert!(ir.contains("store double 4.200000e+01"));
    assert_eq!(compile_and_run(source, "frame_value_return"), "before\n");
}

#[test]
fn compiles_synchronous_break_and_continue() {
    let source = r#"
        function main(): void {
            let index: number = 0;
            let total: number = 0;
            while (index < 5) {
                index = index + 1;
                if (index < 3) {
                    continue;
                }
                total = total + index;
                if (index > 3) {
                    break;
                }
            }
            console.log(total);
        }
    "#;
    assert_eq!(compile_and_run(source, "sync_break_continue"), "7\n");
}

#[test]
fn compiles_labeled_loop_control_and_block_breaks() {
    let source = r#"
        function main(): void {
            let total: number = 0;
            outer: for (let i: number = 0; i < 4; i = i + 1) {
                for (let j: number = 0; j < 4; j = j + 1) {
                    if (j === 1) continue outer;
                    total = total + 1;
                    if (i === 2) break outer;
                }
            }
            done: {
                total = total + 10;
                break done;
                total = 1000;
            }
            console.log(total);
        }
    "#;
    assert_eq!(compile_and_run(source, "labeled_loop_control"), "13\n");
}

#[test]
fn frame_split_supports_break_and_continue_in_while_loop() {
    let source = r#"
        async function main(): Promise<void> {
            let continued: number = 0;
            while (continued < 3) {
                continued = continued + 1;
                await sleep(1);
                continue;
                continued = 100;
            }
            console.log(continued);

            let broken: number = 0;
            while (true) {
                await sleep(1);
                broken = broken + 1;
                break;
                broken = 100;
            }
            console.log(broken);
        }
    "#;
    assert_eq!(compile_and_run(source, "async_break_continue"), "3\n1\n");
}

#[test]
fn frame_split_supports_labeled_nested_loop_control() {
    let source = r#"
        async function main(): Promise<void> {
            let total: number = 0;
            let i: number = 0;
            outer: while (i < 4) {
                i = i + 1;
                let j: number = 0;
                while (j < 3) {
                    j = j + 1;
                    await sleep(1);
                    if (j === 2) continue outer;
                    total = total + 1;
                    if (i === 3) break outer;
                }
            }
            console.log(total);
        }
    "#;
    assert_eq!(compile_and_run(source, "async_labeled_control"), "3\n");
}

#[test]
fn frame_split_supports_nested_break_and_continue() {
    let source = r#"
        async function main(): Promise<void> {
            let index: number = 0;
            let total: number = 0;
            while (index < 5) {
                index = index + 1;
                await sleep(1);
                if (index < 3) {
                    continue;
                }
                if (index > 3) {
                    break;
                }
                total = total + index;
            }
            console.log(total);
            console.log(index);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nested_async_loop_control"),
        "3\n4\n"
    );
}

#[test]
fn frame_split_runs_finally_then_rethrows_child_rejection() {
    let source = r#"
        async function fail(): Promise<void> {
            await sleep(1);
            throw "boom";
        }

        async function main(): Promise<void> {
            console.log("before");
            try {
                await fail();
                console.log("unreachable");
            } finally {
                console.log("finally");
            }
            console.log("also-unreachable");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_rejection_finally"),
        "before\nfinally\n"
    );
}

#[test]
fn frame_split_runs_catch_then_finally_for_child_rejection() {
    let source = r#"
        async function fail(): Promise<void> {
            await sleep(1);
            throw "boom";
        }

        async function main(): Promise<void> {
            try {
                await fail();
            } catch (error) {
                console.log(error);
                await sleep(1);
                console.log("caught");
            } finally {
                console.log("finally");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_catch_finally_rejection"),
        "boom\ncaught\nfinally\ndone\n"
    );
}

#[test]
fn frame_split_runs_finally_before_catch_rethrow() {
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
                throw "wrapped";
            } finally {
                console.log("finally");
            }
        }

        async function main(): Promise<void> {
            await relay();
            console.log("unreachable");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_catch_rethrow_finally"),
        "original\nfinally\n"
    );
}

#[test]
fn frame_split_routes_inner_catch_rethrow_to_outer_catch() {
    let source = r#"
        async function main(): Promise<void> {
            try {
                try {
                    await sleep(1);
                    throw "inner";
                } catch (inner) {
                    console.log(inner);
                    await sleep(1);
                    throw "wrapped";
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
        compile_and_run(source, "nested_async_catch_rethrow"),
        "inner\nwrapped\nouter-caught\ndone\n"
    );
}

#[test]
fn catch_without_a_binding_still_prints_correctly() {
    let source = r#"
        function boom(): number {
            throw "x";
        }
        function main(): void {
            try {
                boom();
            } catch {
                console.log("missing");
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "catch_without_a_binding_still_prints_correctly"),
        "missing\n"
    );
}

