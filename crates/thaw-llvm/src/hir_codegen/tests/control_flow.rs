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

/// A `let`/`var` declared with no initializer at all used to be
/// rejected unconditionally ("`x` needs an initializer"), regardless of
/// its declared type -- a real, common pattern (real example: csv-
/// parse's own streaming API, `let record; while ((record = parser.
/// read()) !== null) { ... }`, mirroring Node's own `Readable` docs).
/// Exercises the two cases that now work: no annotation at all
/// (defaults to `Json`, matching how a bare `any`/`unknown` annotation
/// already lowers) and an explicit annotation whose real-JS initial
/// value (`undefined`) already has a well-defined encoding (`T |
/// undefined`, via the existing `Optional` "absent" representation) --
/// both assigned to only after their declaration, the exact real-world
/// shape. A concrete scalar annotation with no such encoding (`let x:
/// number;`) is still rejected -- see `thaw-hir`'s own `rejects_an_
/// uninitialized_declaration_with_no_default_value`.
#[test]
fn compiles_uninitialized_declarations_assigned_later() {
    let source = r#"
        function main(): void {
            let record;
            let i = 0;
            while (i < 3) {
                record = i * 2;
                i = i + 1;
            }
            console.log(record);

            let maybeName: string | undefined;
            console.log(maybeName === undefined);
            maybeName = "assigned later";
            console.log(maybeName);

            var hoisted;
            hoisted = "hoisted too";
            console.log(hoisted);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "uninitialized_decl"),
        "4\ntrue\nassigned later\nhoisted too\n"
    );
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
fn switch_cases_use_strict_kind_matching_in_source_order() {
    let source = r#"
        function probe(label: string, value: any): any {
            console.log(label);
            return value;
        }
        function main(): void {
            switch (1) {
                case probe("text", "1"): console.log("wrong-text"); break;
                case probe("bool", true): console.log("wrong-bool"); break;
                case probe("number", 1): console.log("matched"); break;
                case probe("late", 1): console.log("wrong-late"); break;
            }
            const dynamic: any = "1";
            switch (dynamic) {
                case 1: console.log("wrong-number"); break;
                case "1": console.log("matched-text"); break;
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "strict_switch_case_kinds"),
        "text\nbool\nnumber\nmatched\nmatched-text\n"
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
fn error_options_preserve_cause() {
    let source = r#"
        function main(): void {
            const cause = new TypeError("root");
            const error = new Error("outer", { cause });
            console.log(error.name, error.message);
            console.log(error.cause.name, error.cause.message);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "error_cause"),
        "Error outer\nTypeError root\n"
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
                console.log(e.stack);
                console.log(e instanceof TypeError);
                console.log(e instanceof Error);
                console.log(e instanceof RangeError);
            }
            try {
                throw "plain string";
            } catch (e) {
                console.log(e.message);
                console.log(e.name);
                console.log(e.stack);
                console.log(e instanceof Error);
                console.log(e instanceof TypeError);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "caught_error_properties"),
        "wrong type\nTypeError\nTypeError: wrong type\ntrue\ntrue\nfalse\nplain string\nError\nError: plain string\nfalse\nfalse\n"
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
fn async_await_catches_a_typed_quickjs_promise_rejection() {
    let source = r#"
        declare function __thaw_typed_js_6c617465724173796e63(): Promise<string>;

        async function main(): Promise<void> {
            loadScript("globalThis.laterAsync = () => { const error = new Error('stopped'); error.name = 'AbortError'; return Promise.reject(error); };");
            try {
                await __thaw_typed_js_6c617465724173796e63();
            } catch (error) {
                console.log(error.name, error.message);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "typed_quickjs_async_rejection"),
        "AbortError `laterAsync`'s promise rejected: stopped\n"
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
fn rejected_await_in_finalizer_skips_its_remaining_statements() {
    let source = r#"
        async function main(): Promise<void> {
            let cleanup = 0;
            try {
                try {
                    return;
                } finally {
                    await Promise.reject("failed");
                    cleanup++;
                }
            } catch (error) {
                console.log(error, cleanup);
            }
        }
    "#;
    assert_eq!(compile_and_run(source, "async_finalizer_rejection_guard"), "failed 0\n");
}

#[test]
fn conditional_and_loop_exit_finalizers_bypass_source_catch_after_await() {
    let source = r#"
        let wrong = 0;
        async function conditional(flag: boolean): Promise<number> {
            try {
                if (flag) return 1;
                return 2;
            } catch (error) {
                wrong++;
                return 3;
            } finally {
                await Promise.reject("override");
            }
        }
        async function loop(flag: boolean): Promise<number> {
            try {
                while (flag) return 1;
                return 2;
            } catch (error) {
                wrong++;
                return 3;
            } finally {
                await Promise.reject("override");
            }
        }
        async function main(): Promise<void> {
            try { await conditional(true); } catch (error) { console.log(error, wrong); }
            try { await conditional(false); } catch (error) { console.log(error, wrong); }
            try { await loop(true); } catch (error) { console.log(error, wrong); }
        }
    "#;
    assert_eq!(compile_and_run(source, "nested_exit_finalizer_await_guard"),
        "override 0\noverride 0\noverride 0\n");
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

#[test]
fn throwing_a_non_string_value_coerces_it_to_a_string() {
    // The exception channel is a single tagged string end to end (see
    // `new Error(...)`'s lowering); a thrown number, boolean, or plain
    // object used to be stored into that `i8*` slot as whatever raw bits
    // it happened to have and crash the moment anything tried to read it
    // as a string. Coercing at the `throw` site instead makes every
    // supported thrown type safe, using the same string conversion
    // `String(value)` already uses.
    let source = r#"
        function main(): void {
            try {
                throw 42;
            } catch (e) {
                console.log(e);
            }
            try {
                throw true;
            } catch (e) {
                console.log(e);
            }
            try {
                throw { code: 42, reason: "bad" };
            } catch (e) {
                console.log(e);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "throw_non_string_coerces"),
        "42\ntrue\n[object Object]\n"
    );
}

#[test]
fn throwing_an_unsupported_value_is_a_compile_error_not_a_crash() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            throw Promise.resolve(1);
        }"#,
    )
    .unwrap();
    let error = thaw_hir::lower_module(&module).unwrap_err();
    assert!(error.contains("string conversion") || error.contains("Promise"), "{error}");
}

/// `const g: any = gen();` (coercing a generator's own internal
/// "producer" function -- thaw's own ABI, first parameter an internal
/// `i64` resume point -- into a dynamic `any` value) used to fall
/// through into the generic native-closure-to-`JsValue` wrapping path,
/// which has no way to expose that `i64` parameter to real JS code and
/// failed confusingly deep inside dynamic-call argument decoding:
/// "unsupported dynamic result value I64", nowhere near the actual
/// coercion. `for (const v of gen())` used *directly* (no intermediate
/// `any` storage) was and remains unaffected -- `Stmt::ForOf` recognizes
/// the same producer shape itself and handles it natively, never
/// reaching this coercion at all.
#[test]
fn coercing_a_generator_to_a_dynamic_value_is_a_clear_compile_error() {
    let module = thaw_parser::parse_typescript(
        r#"function* gen(): Generator<any> {
            yield 1;
        }
        function main(): void {
            const g: any = gen();
        }"#,
    )
    .unwrap();
    let error = thaw_hir::lower_module(&module).unwrap_err();
    assert!(error.contains("Generator"), "{error}");
    assert!(!error.contains("I64"), "{error}");
}

#[test]
fn a_non_tail_throw_still_skips_the_rest_of_every_caller() {
    let source = r#"
        function boom(): number {
            throw "boom";
        }
        function middle(): number {
            const x = boom();
            console.log("must not print: " + x);
            return x + 1;
        }
        function main(): void {
            try {
                const result = middle() + 100;
                console.log("must not print: " + result);
            } catch (e) {
                console.log("caught: " + e);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "non_tail_throw_propagation"),
        "caught: boom\n"
    );
}

#[test]
fn promise_resolve_produces_an_already_fulfilled_promise() {
    let source = r#"
        async function main(): Promise<void> {
            const value = await Promise.resolve(42);
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "promise_resolve_value"), "42\n");
}

#[test]
fn promise_reject_carries_a_tagged_error_through_await() {
    let source = r#"
        async function main(): Promise<void> {
            try {
                await Promise.reject(new TypeError("bad promise"));
            } catch (e) {
                console.log(e.message);
                console.log(e.name);
                console.log(e instanceof TypeError);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_reject_tagged_error"),
        "bad promise\nTypeError\ntrue\n"
    );
}

#[test]
fn promise_resolve_assimilates_an_existing_promise() {
    let source = r#"
        async function later(): Promise<number> {
            return 7;
        }
        async function main(): Promise<void> {
            const value = await Promise.resolve(later());
            console.log(value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_resolve_assimilates_a_promise"),
        "7\n"
    );
}

#[test]
fn promise_all_rejects_when_one_element_is_promise_reject() {
    let source = r#"
        async function main(): Promise<void> {
            try {
                await Promise.all([Promise.resolve(1), Promise.reject<number>("nope")]);
                console.log("must not print");
            } catch (e) {
                console.log("caught: " + e);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_with_reject"),
        "caught: nope\n"
    );
}

#[test]
fn user_class_extends_error_exposes_message_name_instanceof() {
    let source = r#"
        class MyError extends Error {
            code: number;
            constructor(message: string, code: number) {
                super(message);
                this.code = code;
            }
        }
        function main(): void {
            const built = new MyError("bad thing", 42);
            console.log(built.code);
            console.log(built instanceof MyError);
            try {
                throw built;
            } catch (e) {
                console.log(e.message);
                console.log(e.name);
                console.log(e instanceof Error);
                console.log(e instanceof MyError);
                console.log(e instanceof TypeError);
            }
        }
    "#;
    // `.name` defaults to `"Error"` (the nearest native ancestor), not
    // the subclass's own name -- confirmed against real Node: a class
    // extending `Error` that never assigns `this.name` itself inherits
    // `Error.prototype.name`, exactly like real JavaScript's own
    // prototype chain (see `lower/invocations/calls.rs`'s `super(...)`
    // handling and `lower/declarations.rs`'s implicit-constructor path,
    // both of which now default `this.name` the same way).
    assert_eq!(
        compile_and_run(source, "user_class_extends_error"),
        "42\ntrue\nbad thing\nError\ntrue\ntrue\nfalse\n"
    );
}

#[test]
fn error_subclass_inherits_an_implicit_message_constructor() {
    let source = r#"
        class AppError extends Error {
            code: number = 1;
        }
        function main(): void {
            const error = new AppError("failed");
            console.log(error.message);
            console.log(error.code);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "implicit_error_constructor"),
        "failed\n1\n"
    );
}

#[test]
fn multi_level_error_subclass_instanceof_matches_every_ancestor() {
    let source = r#"
        class MyError extends Error {
            constructor(message: string) {
                super(message);
            }
        }
        class Sub extends MyError {
            constructor(message: string) {
                super(message);
            }
        }
        function main(): void {
            try {
                throw new Sub("deep failure");
            } catch (e) {
                console.log(e.message);
                console.log(e.name);
                console.log(e instanceof Sub);
                console.log(e instanceof MyError);
                console.log(e instanceof Error);
                console.log(e instanceof TypeError);
            }
        }
    "#;
    // `.name` defaults to `"Error"` here too, for the same reason as
    // `user_class_extends_error_exposes_message_name_instanceof` above
    // -- confirmed against real Node even through this two-level chain.
    assert_eq!(
        compile_and_run(source, "multi_level_error_subclass"),
        "deep failure\nError\ntrue\ntrue\ntrue\nfalse\n"
    );
}

/// `this.name = "..."` inside an Error-subclass constructor used to fail
/// to compile outright ("cannot assign to `.name`") -- `name` wasn't in
/// the class's inherited field list the way `message` already was.
/// Fixed by adding `name` as an inherited `Str` field (defaulting to the
/// nearest native ancestor's own name, see the two tests above) and
/// threading its *runtime* value through the tagged-string throw
/// mechanism (`lower/expressions/coercions.rs`) as an override, so an
/// explicit assignment here is what a `catch` block actually observes,
/// not just a field write with no effect on the thrown/caught value.
/// Also exercises `.toString()` (the method-call form, not `String(e)`)
/// to prove it stays consistent with the override too.
#[test]
fn explicit_this_name_assignment_overrides_the_default_and_is_observable_when_caught() {
    let source = r#"
        class MyError extends Error {
            constructor(message: string) {
                super(message);
                this.name = "MyError";
            }
        }
        function main(): void {
            try {
                throw new MyError("oops");
            } catch (e) {
                console.log(e.name);
                console.log(e.message);
                console.log(e.toString());
                console.log(e.stack);
                console.log(e instanceof MyError);
                console.log(e instanceof Error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "explicit_error_name_assignment"),
        "MyError\noops\nMyError: oops\nMyError: oops\ntrue\ntrue\n"
    );
}

#[test]
fn instanceof_narrows_a_caught_error_to_its_object_type() {
    let source = r#"
        class MyError extends Error {
            code: number;
            constructor(message: string, code: number) {
                super(message);
                this.code = code;
            }
        }
        function main(): void {
            try {
                throw new MyError("bad thing", 42);
            } catch (e) {
                if (e instanceof MyError) {
                    console.log(e.code);
                }
                console.log(e.message);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "caught_error_field_via_as_cast"),
        "42\nbad thing\n"
    );
}

#[test]
fn instanceof_guard_skips_object_access_for_an_unrelated_thrown_value() {
    let source = r#"
        class MyError extends Error {
            code: number;
            constructor(message: string, code: number) {
                super(message);
                this.code = code;
            }
        }
        function main(): void {
            try {
                throw "plain string";
            } catch (e) {
                if (e instanceof MyError) {
                    console.log(e.code);
                } else {
                    console.log("not a MyError");
                }
                console.log(e.message);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(
            source,
            "as_cast_guarded_by_instanceof"
        ),
        "not a MyError\nplain string\n"
    );
}

#[test]
fn generators_close_after_uncaught_throw_and_keep_caught_throws_resumable() {
    let source = r#"
        function* syncValues(): Generator<number> { yield 1; throw "sync boom"; }
        async function* asyncValues(): AsyncGenerator<number> { yield 2; await sleep(1); throw "async boom"; }
        function* caught(): Generator<number> { try { throw "caught"; } catch (e) { yield 3; } yield 4; }
        async function main(): Promise<void> {
            const s = syncValues(); console.log(s.next().value);
            try { s.next(); } catch (e) { console.log(e); }
            console.log(s.next().done, s.next().value);
            const a = asyncValues(); console.log((await a.next()).value);
            try { await a.next(); } catch (e) { console.log(e); }
            console.log((await a.next()).done, (await a.next()).value);
            const c = caught(); console.log(c.next().value, c.next().value, c.next().done);
        }
    "#;
    assert_eq!(compile_and_run(source, "generator_uncaught_completion"),
        "1\nsync boom\ntrue undefined\n2\nasync boom\ntrue undefined\n3 4 true\n");
}

#[test]
fn async_generators_close_without_erasing_caught_rejection_metadata() {
    let source = r#"
        async function* typed(): AsyncGenerator<number> {
            yield 1;
            await sleep(1);
            throw new TypeError("typed boom");
        }
        async function* numeric(): AsyncGenerator<number> {
            yield 2;
            await sleep(1);
            throw 42;
        }
        async function failNested(): Promise<void> {
            await sleep(1);
            throw new TypeError("nested boom");
        }
        async function* nested(): AsyncGenerator<number> {
            yield 3;
            try { await failNested(); }
            catch (error) {
                try { throw "inner"; } catch (ignored) {}
                throw error;
            }
        }
        async function* reassigned(): AsyncGenerator<number> {
            yield 4;
            try { await failNested(); }
            catch (error) { error = "replacement"; throw error; }
        }
        async function main(): Promise<void> {
            const a = typed();
            console.log((await a.next()).value);
            try { await a.next(); }
            catch (error) { console.log(error instanceof TypeError, error.message); }
            console.log((await a.next()).done);
            const b = numeric();
            console.log((await b.next()).value);
            try { await b.next(); }
            catch (error) { console.log(typeof error, (error as number) + 1); }
            console.log((await b.next()).done);
            const c = nested();
            console.log((await c.next()).value);
            try { await c.next(); }
            catch (error) { console.log(error instanceof TypeError, error.message); }
            console.log((await c.next()).done);
            const d = reassigned();
            console.log((await d.next()).value);
            try { await d.next(); }
            catch (error) { console.log(error instanceof TypeError, error); }
            console.log((await d.next()).done);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_generator_uncaught_typed_completion"),
        "1\ntrue typed boom\ntrue\n2\nnumber 43\ntrue\n3\ntrue nested boom\ntrue\n4\nfalse replacement\ntrue\n"
    );
}

#[test]
fn guarded_async_direct_throws_keep_string_and_typed_rejection_metadata() {
    let source = r#"
        async function stringFailure(): Promise<void> {
            try { await sleep(1); throw "seed"; }
            catch (ignored) { throw "direct text"; }
        }
        async function typedFailure(): Promise<void> {
            try { await sleep(1); throw "seed"; }
            catch (ignored) { throw new TypeError("direct typed"); }
        }
        async function main(): Promise<void> {
            try { await stringFailure(); }
            catch (error) { console.log(typeof error, error === "direct text"); }
            try { await typedFailure(); }
            catch (error) { console.log(error instanceof TypeError, error.message); }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "guarded_async_direct_throw_metadata"),
        "string true\ntrue direct typed\n"
    );
}


#[test]
fn error_constructor_messages_with_control_markers_are_bound_once() {
    let source = r#"
        class MarkerError extends Error {
            constructor(message: string) { super(message); }
        }
        let calls = 0;
        function message(): string {
            calls++;
            return "a\u0001b\u0002c\u0003d\u0004e\u0005f\u0006g";
        }
        function main(): void {
            const expected = "a\u0001b\u0002c\u0003d\u0004e\u0005f\u0006g";
            try { throw new Error(message()); }
            catch (e) { console.log(e.message === expected); }
            try { throw Error(message()); }
            catch (e) { console.log(e.message === expected); }
            try { throw new MarkerError(message()); }
            catch (e) { console.log(e.message === expected, e instanceof MarkerError); }
            console.log(calls);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "error_constructor_control_messages"),
        "true\ntrue\ntrue true\n3\n"
    );
}

#[test]
fn using_disposes_only_acquired_resources_once_on_return_failure() {
    let source = r#"
        function make(name: string) {
            console.log("acquire " + name);
            if (name === "b") throw new Error("acquire b");
            return { [Symbol.dispose]() { console.log("dispose " + name); } };
        }
        function partial(): void {
            using a = make("a"), b = make("b"), c = make("c");
        }
        function failing(name: string) {
            return { [Symbol.dispose]() {
                console.log("dispose " + name);
                if (name === "b") throw new Error("dispose b");
            } };
        }
        function early(): number {
            using a = failing("a"), b = failing("b");
            return 7;
        }
        function main(): void {
            try { partial(); } catch (error) { console.log(error.message); }
            try { console.log(early()); } catch (error) { console.log(error.message); }
        }
    "#;
    assert_eq!(compile_and_run(source, "using_acquired_once"),
        "acquire a\nacquire b\ndispose a\nacquire b\ndispose b\ndispose a\ndispose b\n");
}

#[test]
fn using_iteration_disposal_failure_does_not_repeat_after_continue() {
    let source = r#"
        function make(name: string) {
            return { [Symbol.dispose]() {
                console.log("dispose " + name);
                throw new Error(name);
            } };
        }
        function main(): void {
            try {
                for (using item of [make("first"), make("second")]) {
                    console.log("body");
                    continue;
                }
            } catch (error) { console.log("caught " + error.message); }
        }
    "#;
    assert_eq!(compile_and_run(source, "using_iteration_once"),
        "body\ndispose first\ncaught first\n");
}

#[test]
fn exit_finalizer_throw_bypasses_every_exited_catch() {
    let source = r#"
        function exit(): number {
            try {
                try { return 1; }
                catch (inner) { console.log("wrong-inner"); return 2; }
            } catch (outer) {
                console.log("wrong-outer");
                return 3;
            } finally {
                console.log("cleanup");
                throw "override";
            }
        }
        function main(): void {
            try { console.log(exit()); }
            catch (error) { console.log(error); }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "exit_finalizer_bypasses_all_catches"),
        "cleanup\noverride\n",
    );
}

#[test]
fn finally_keeps_evaluated_return_and_break_order() {
    let source = r#"
        function value(): number {
            let state: number = 1;
            try { return state; }
            finally { state = 2; }
        }
        function main(): void {
            console.log(value());
            let state: number = 0;
            while (true) {
                try { break; }
                finally { state = 3; }
            }
            console.log(state);
        }
    "#;
    assert_eq!(compile_and_run(source, "finally_exit_evaluation_order"), "1\n3\n");
}

#[test]
fn frame_split_finalizer_rejection_bypasses_exited_catches() {
    let source = r#"
        async function value(): Promise<number> {
            try {
                try { await sleep(1); return 1; }
                catch (inner) { console.log("wrong-inner"); return 2; }
            } catch (outer) {
                console.log("wrong-outer");
                return 3;
            } finally {
                await sleep(1);
                console.log("cleanup");
                throw "override";
            }
        }
        async function main(): Promise<void> {
            try { await value(); }
            catch (error) { console.log(error); }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_exit_finalizer_bypasses_all_catches"),
        "cleanup\noverride\n",
    );
}

#[test]
fn throwing_source_catch_runs_finalizer_once_with_original_failure() {
    let source = r#"
        let cleanups: number = 0;
        function fail(): number { throw "second"; }
        function run(): number {
            try { throw "first"; }
            catch (first) { fail(); return 0; }
            finally { cleanups += 1; }
        }
        function main(): void {
            try { run(); }
            catch (error) { console.log(error); }
            console.log(cleanups);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "throwing_catch_finalizer_once"),
        "second\n1\n",
    );
}

#[test]
fn source_catch_explicit_throw_does_not_rerun_finalizer_guard() {
    let source = r#"
        let cleanups: number = 0;
        function run(): void {
            try { throw "first"; }
            catch (first) { throw "second"; }
            finally { cleanups += 1; }
        }
        function main(): void {
            try { run(); }
            catch (error) { console.log(error); }
            console.log(cleanups);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "catch_rethrow_finalizer_once"),
        "second\n1\n",
    );
}
