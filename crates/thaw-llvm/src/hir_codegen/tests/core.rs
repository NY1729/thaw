/// Full pipeline smoke test: TS source -> thaw-parser -> thaw-hir ->
/// this module -> a real object file -> linked into a native binary via
/// the system `cc` -> executed as a subprocess. This is the literal
/// Phase 0 "numbers, strings, functions -> LLVM IR, Hello World binary"
/// deliverable, not a mock of it.
#[test]
fn compiles_and_runs_a_native_hello_world_binary() {
    let source = r#"
        function add(a: number, b: number): number {
            return a + b;
        }

        function main(): void {
            console.log("Hello, Thaw!");
            console.log(add(2, 3));
        }
    "#;

    assert_eq!(compile_and_run(source, "hello"), "Hello, Thaw!\n5\n");
}

#[test]
fn console_log_accepts_multiple_arguments_after_evaluating_them_in_order() {
    let source = r#"
        function first(): string {
            console.log("first");
            return "A";
        }
        function second(): number {
            console.log("second");
            return 2;
        }
        function main(): void {
            const missing: number | undefined = undefined;
            const present: number | undefined = 3;
            console.log();
            console.log("value", 1, true, null, undefined);
            console.log(first(), second(), "done");
            console.log("optional", missing);
            console.log("optional", present);
        }
    "#;

    assert_eq!(
        compile_and_run(source, "variadic_console_log"),
        "\nvalue 1 true null undefined\nfirst\nsecond\nA 2 done\noptional undefined\noptional 3\n"
    );
}

#[test]
fn console_log_serializes_arrays_objects_and_json_values() {
    let source = r#"
        interface Item {
            name: string;
            active: boolean;
            scores: number[];
        }
        function main(): void {
            const numbers: number[] = [1, 2];
            const strings: string[] = ["a", "b"];
            const flags: boolean[] = [true, false];
            const matrix: number[][] = [[1, 2], [3]];
            const item: Item = { name: "thaw", active: true, scores: [4, 5] };
            const maybe: Item | undefined = item;
            console.log(numbers, strings, flags);
            console.log(matrix);
            console.log(item);
            console.log(maybe);
            console.log(JSON.parse("{\"nested\":[1,true,null]}"));
        }
    "#;

    assert_eq!(
        compile_and_run(source, "structured_console_log"),
        "[1,2] [\"a\",\"b\"] [true,false]\n[[1,2],[3]]\n{\"name\":\"thaw\",\"active\":true,\"scores\":[4,5]}\n{\"name\":\"thaw\",\"active\":true,\"scores\":[4,5]}\n{\"nested\":[1,true,null]}\n"
    );
}

#[test]
fn console_log_serializes_typed_tuples_in_all_tagged_positions() {
    let source = r#"
        interface Point { x: number; label: string; }
        function main(): void {
            const tuple: [number, string, boolean, Point, number[]] =
                [1, "two", true, { x: 3, label: "p" }, [4, 5]];
            const nested: [[number, string], [boolean, Point]] =
                [[6, "seven"], [false, { x: 8, label: "q" }]];
            const maybe: [string, number] | undefined = ["nine", 10];
            const missing: [string, number] | undefined = undefined;
            const mixed: [number, string] | string = [11, "twelve"];
            console.log(tuple);
            console.log(nested);
            console.log(maybe);
            console.log(missing);
            console.log(mixed);
        }
    "#;

    assert_eq!(
        compile_and_run(source, "tuple_console_log"),
        "[1,\"two\",true,{\"x\":3,\"label\":\"p\"},[4,5]]\n[[6,\"seven\"],[false,{\"x\":8,\"label\":\"q\"}]]\n[\"nine\",10]\nundefined\n[11,\"twelve\"]\n"
    );
}

#[test]
fn console_log_serializes_tagged_values_inside_collections() {
    let source = r#"
        interface Tagged {
            optional?: number;
            nullable: string | null;
            nullish: boolean | null | undefined;
        }
        function main(): void {
            const optional: (number | undefined)[] = [1, undefined, 3];
            const nullable: (string | null)[] = ["a", null, "c"];
            const nullish: (boolean | null | undefined)[] = [true, null, undefined];
            const tuple: [number | undefined, string | null, boolean | null | undefined] =
                [undefined, null, undefined];
            const absent: Tagged = { nullable: null, nullish: undefined };
            const present: Tagged = { optional: 2, nullable: "x", nullish: null };
            console.log(optional);
            console.log(nullable);
            console.log(nullish);
            console.log(tuple);
            console.log(absent);
            console.log(present);
        }
    "#;

    assert_eq!(
        compile_and_run(source, "tagged_collection_console"),
        "[1,null,3]\n[\"a\",null,\"c\"]\n[true,null,null]\n[null,null,null]\n{\"nullable\":null}\n{\"optional\":2,\"nullable\":\"x\",\"nullish\":null}\n"
    );
}

#[test]
fn console_methods_use_their_node_compatible_output_streams() {
    let source = r#"
        interface Detail { code: number; }
        function value(): string {
            console.log("evaluated");
            return "A";
        }
        function main(): void {
            const detail: Detail = { code: 7 };
            console.info("info", 1);
            console.debug("debug", true);
            console.warn(value(), "warning");
            console.error("error", detail);
        }
    "#;

    let (stdout, stderr) = compile_and_run_output(source, "console_output_streams");
    assert_eq!(stdout, "info 1\ndebug true\nevaluated\n");
    assert_eq!(stderr, "A warning\nerror {\"code\":7}\n");
}

#[test]
fn console_assert_evaluates_all_arguments_and_only_reports_falsy_conditions() {
    let source = r#"
        interface Detail { code: number; }
        function condition(value: boolean): boolean {
            console.log("condition", value);
            return value;
        }
        function message(value: string): string {
            console.log("message", value);
            return value;
        }
        function main(): void {
            const detail: Detail = { code: 7 };
            console.assert(condition(true), message("ignored"));
            console.assert(false);
            console.assert(condition(false), message("failed"), 2, detail);
            console.assert(0, "zero");
            console.assert(1, "one");
            console.assert("", "empty");
            console.assert("value", "string");
            console.assert(JSON.parse("false"), "json");
            console.assert(detail, "object");
            console.assert();
        }
    "#;

    let (stdout, stderr) = compile_and_run_output(source, "console_assert");
    assert_eq!(
        stdout,
        "condition true\nmessage ignored\ncondition false\nmessage failed\n"
    );
    assert_eq!(
        stderr,
        "Assertion failed\nAssertion failed: failed 2 {\"code\":7}\nAssertion failed: zero\nAssertion failed: empty\nAssertion failed: json\nAssertion failed\n"
    );
}

#[test]
fn console_log_safely_formats_functions_promises_and_dynamic_handles() {
    let source = r#"
        async function delayed(): Promise<number> {
            await sleep(1);
            return 42;
        }
        function main(): void {
            const callback: (value: number) => number =
                (value: number): number => value + 1;
            const optional: ((value: number) => number) | undefined = callback;
            const mixed: ((value: number) => number) | string = callback;
            const pending: Promise<number> = delayed();
            console.log(callback);
            console.log(optional);
            console.log(mixed);
            console.log(pending);

            loadScript("globalThis.consoleNumber = 42; globalThis.consoleObject = { value: 7 };");
            const numberHandle: JsValue = getDynamicValue("consoleNumber");
            const objectHandle: JsValue = getDynamicValue("consoleObject");
            console.log(numberHandle, objectHandle);
        }
    "#;

    assert_eq!(
        compile_and_run(source, "opaque_console_values"),
        "[Function]\n[Function]\n[Function]\nPromise { <pending> }\n42 [object Object]\n"
    );
}

#[test]
fn top_level_destructuring_supports_nested_defaults_rests_and_runtime_keys() {
    let source = r#"
        interface Source {
            point: { x: number; y: number };
            label: string | undefined;
            extra: number;
        }
        let calls: number = 0;
        function objectSource(): Source {
            calls += 1;
            return { point: { x: 1, y: 2 }, label: undefined, extra: 3 };
        }
        function arraySource(): number[] {
            calls += 1;
            return [4, 5, 6, 7];
        }
        const {
            point: { x, y },
            label = "fallback",
            ...remaining
        }: Source = objectSource();
        const [first, , third = 30, ...tail]: number[] = arraySource();
        let keyCalls: number = 0;
        function runtimeKey(): string {
            keyCalls += 1;
            return "chosen";
        }
        const dictionary: Record<string, number> = { chosen: 8, kept: 9 };
        const {
            [runtimeKey()]: chosen,
            ...dictionaryRest
        }: Record<string, number> = dictionary;

        function main(): void {
            console.log(calls, x, y, label, remaining.extra);
            console.log(first, third, tail, keyCalls, chosen, dictionaryRest);
        }
    "#;

    assert_eq!(
        compile_and_run(source, "top_level_destructuring"),
        "2 1 2 fallback 3\n4 6 [7] 1 8 {\"kept\":9}\n"
    );
}

#[test]
fn local_dictionary_destructuring_supports_computed_keys_and_rest() {
    let source = r#"
        let sourceCalls: number = 0;
        let keyCalls: number = 0;
        function source(): Record<string, number> {
            sourceCalls += 1;
            return { selected: 1, first: 2, second: 3 };
        }
        function key(): string {
            keyCalls += 1;
            return "selected";
        }
        function run(): void {
            const {
                [key()]: selected,
                first,
                missing = 4,
                ...rest
            }: Record<string, number> = source();
            console.log(sourceCalls, keyCalls, selected, first, missing, rest);
            console.log(source());
        }
        function main(): void {
            run();
        }
    "#;

    assert_eq!(
        compile_and_run(source, "local_dictionary_destructuring"),
        "1 1 1 2 4 {\"second\":3}\n{\"selected\":1,\"first\":2,\"second\":3}\n"
    );
}

#[test]
fn dictionary_destructuring_assignment_supports_computed_keys_defaults_and_rest() {
    let source = r#"
        let assignmentSourceCalls: number = 0;
        let assignmentKeyCalls: number = 0;
        function assignmentSource(): Record<string, number> {
            assignmentSourceCalls += 1;
            return { selected: 1, kept: 2 };
        }
        function assignmentKey(): string {
            assignmentKeyCalls += 1;
            return "selected";
        }
        function main(): void {
            let selected: number = 0;
            let missing: number = 0;
            let rest: Record<string, number> = {};
            ({ [assignmentKey()]: selected, missing = 4, ...rest } = assignmentSource());
            console.log(
                assignmentSourceCalls,
                assignmentKeyCalls,
                selected,
                missing,
                rest
            );
            console.log(assignmentSource());
        }
    "#;

    assert_eq!(
        compile_and_run(source, "dictionary_destructuring_assignment"),
        "1 1 1 4 {\"kept\":2}\n{\"selected\":1,\"kept\":2}\n"
    );
}

#[test]
fn structured_dictionary_values_support_reads_and_destructuring() {
    let source = r#"
        interface Item { value: number; label: string; }
        interface RichItem {
            tags: string[];
            matrix: number[][];
            metadata: Record<string, number>;
        }
        interface TaggedItem {
            optional?: number;
            nullable: number | null;
            nullish: number | null | undefined;
        }
        function items(): Record<string, Item> {
            return {
                selected: { value: 1, label: "one" },
                kept: { value: 2, label: "two" }
            };
        }
        function arrays(): Record<string, number[]> {
            return { selected: [3, 4], kept: [5] };
        }
        function labels(): Record<string, string[]> {
            return { selected: ["a", "b"], kept: ["c"] };
        }
        function flags(): Record<string, boolean[]> {
            return { selected: [true, false], kept: [true] };
        }
        function itemArrays(): Record<string, Item[]> {
            return {
                selected: [
                    { value: 6, label: "six" },
                    { value: 7, label: "seven" }
                ],
                kept: [{ value: 8, label: "eight" }]
            };
        }
        function matrices(): Record<string, number[][]> {
            return { selected: [[9, 10], [11]], kept: [[12]] };
        }
        function richItems(): Record<string, RichItem> {
            return {
                selected: {
                    tags: ["x", "y"],
                    matrix: [[13], [14, 15]],
                    metadata: { score: 16 }
                }
            };
        }
        function tuples(): Record<string, [string, number, Item]> {
            return {
                selected: ["tuple", 17, { value: 18, label: "eighteen" }]
            };
        }
        function taggedItems(): Record<string, TaggedItem> {
            return {
                selected: { optional: 19, nullable: null, nullish: undefined },
                other: { nullable: 20, nullish: null }
            };
        }
        function main(): void {
            const direct: Item = items().selected;
            const {
                selected: { value, label },
                ...itemRest
            }: Record<string, Item> = items();
            let selected: number[] = [];
            let arrayRest: Record<string, number[]> = {};
            ({ selected, ...arrayRest } = arrays());
            const selectedLabels: string[] = labels().selected;
            const { selected: selectedFlags }: Record<string, boolean[]> = flags();
            const selectedItems: Item[] = itemArrays().selected;
            let selectedMatrix: number[][] = [];
            ({ selected: selectedMatrix } = matrices());
            const rich: RichItem = richItems().selected;
            const tuple: [string, number, Item] = tuples().selected;
            const tagged: TaggedItem = taggedItems().selected;
            const taggedOther: TaggedItem = taggedItems().other;
            console.log(direct.value, direct.label, value, label, itemRest);
            console.log(selected, arrayRest);
            console.log(selectedLabels, selectedFlags);
            console.log(selectedItems[0].value, selectedItems[1].label, selectedMatrix);
            console.log(rich.tags, rich.matrix, rich.metadata.score);
            console.log(tuple[0], tuple[1], tuple[2].label);
            console.log(tagged.optional, tagged.nullable, tagged.nullish);
            console.log(taggedOther.optional, taggedOther.nullable, taggedOther.nullish);
        }
    "#;

    assert_eq!(
        compile_and_run(source, "structured_dictionary_destructuring"),
        "1 one 1 one {\"kept\":{\"value\":2,\"label\":\"two\"}}\n[3,4] {\"kept\":[5]}\n[\"a\",\"b\"] [true,false]\n6 seven [[9,10],[11]]\n[\"x\",\"y\"] [[13],[14,15]] 16\ntuple 17 eighteen\n19 null undefined\nundefined 20 null\n"
    );
}

#[test]
fn dictionary_reads_restore_optional_nullable_and_nullish_values() {
    let source = r#"
        let keyCalls: number = 0;
        function key(): string {
            keyCalls += 1;
            return "present";
        }
        function optionalValues(): Record<string, number | undefined> {
            return { present: 1, missing: undefined };
        }
        function nullableValues(): Record<string, number | null> {
            return { present: 2, empty: null };
        }
        function nullishValues(): Record<string, number | null | undefined> {
            return { present: 3, empty: null, missing: undefined };
        }
        function main(): void {
            const optional: Record<string, number | undefined> = optionalValues();
            const nullable: Record<string, number | null> = nullableValues();
            const nullish: Record<string, number | null | undefined> = nullishValues();
            console.log(optional[key()], optional.missing, keyCalls);
            console.log(nullable.present, nullable.empty, nullable.absent);
            console.log(nullish.present, nullish.empty, nullish.missing);
            const {
                present: optionalPresent,
                missing: optionalMissing
            }: Record<string, number | undefined> = optionalValues();
            let nullishPresent: number | null | undefined = undefined;
            let nullishEmpty: number | null | undefined = undefined;
            let nullishMissing: number | null | undefined = null;
            ({
                present: nullishPresent,
                empty: nullishEmpty,
                missing: nullishMissing
            } = nullishValues());
            console.log(optionalPresent, optionalMissing);
            console.log(nullishPresent, nullishEmpty, nullishMissing);
        }
    "#;

    assert_eq!(
        compile_and_run(source, "tagged_dictionary_reads"),
        "1 undefined 1\n2 null null\n3 null undefined\n1 undefined\n3 null undefined\n"
    );
}

#[test]
fn compiles_and_calls_a_typed_non_capturing_arrow_function() {
    let source = r#"
        function main(): void {
            const increment: (value: number) => number =
                (value: number): number => value + 1;
            console.log(increment(41));
        }
    "#;
    assert_eq!(compile_and_run(source, "typed_arrow"), "42\n");
}

#[test]
fn compiles_captured_and_nested_arrow_functions() {
    let source = r#"
        function main(): void {
            const base: number = 40;
            const add = (value: number): number => base + value;
            const make = (captured: number): (value: number) => number =>
                (value: number): number => captured + value;
            const nested: (value: number) => number = make(20);
            console.log(add(2));
            console.log(nested(22));
        }
    "#;
    assert_eq!(compile_and_run(source, "captured_arrow"), "42\n42\n");
}

#[test]
fn closures_share_mutable_bindings_with_their_outer_scope() {
    let source = r#"
        function main(): void {
            let count: number = 1;
            const increment = (): number => {
                count = count + 1;
                return count;
            };
            count = 40;
            console.log(increment());
            console.log(increment());
            console.log(count);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "mutable_captured_arrow"),
        "41\n42\n42\n"
    );
}

#[test]
fn calls_rest_function_values_across_typed_boundaries() {
    let source = r#"
        interface Holder { run: (prefix: string, ...values: string[]) => string; }
        function pass(
            callback: (prefix: string, ...values: string[]) => string
        ): (prefix: string, ...values: string[]) => string {
            return callback;
        }
        function main(): void {
            const combine: (prefix: string, ...values: string[]) => string =
                (prefix: string, ...values: string[]): string =>
                    prefix + values.join("|");
            const through = pass(combine);
            const holder: Holder = { run: through };
            const tail: [string, string] = ["b", "c"];
            const applied: [string, string, string] = ["apply:", "x", "y"];
            const bound = through.bind(null, "bound:", "a");
            console.log(through("values:", "a", ...tail));
            console.log(through("empty:"));
            console.log(holder.run("object:", ...tail));
            console.log(through.call(null, "call:", "x", "y"));
            console.log(through.apply(null, applied));
            console.log(bound("b", "c"));
            console.log(bound.call("ignored", "d"));
            console.log(through.bind(null, "immediate:", "a")("b"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "rest_function_boundary"),
        "values:a|b|c\nempty:\nobject:b|c\ncall:x|y\napply:x|y\nbound:a|b|c\nbound:a|d\nimmediate:a|b\n"
    );
}

#[test]
fn recursively_calls_named_local_function_values() {
    let source = r#"
        function pass(callback: (value: number) => number): (value: number) => number {
            return callback;
        }
        function main(): void {
            const multiplier = 2;
            const factorial = function factorial(value: number): number {
                if (value <= 1) { return 1; }
                return value * factorial(value - 1);
            };
            let calls = 0;
            const fibonacci: (value: number) => number = function recurse(value) {
                calls += 1;
                if (value <= 1) { return value; }
                return recurse(value - 1) + recurse(value - 2);
            };
            const scaled = function inner(value: number): number {
                if (value <= 0) { return 0; }
                return multiplier + inner(value - 1);
            };
            console.log(factorial(5));
            console.log(fibonacci(6));
            console.log(calls);
            console.log(scaled(4));
            const passed = pass(factorial);
            const bound = passed.bind(null, 4);
            console.log(passed.call(null, 4));
            console.log(bound());
            console.log(pass(factorial).bind(null, 3)());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "recursive_named_local_functions"),
        "120\n8\n25\n8\n24\n24\n6\n"
    );
}

#[test]
fn calls_function_values_with_explicit_this_across_function_boundaries() {
    let source = r#"
        function pass(callback: (value: number) => number): (value: number) => number {
            return callback;
        }
        function make(): (value: number) => number {
            console.log("target");
            return (value: number): number => value + 1;
        }
        function passBinary(callback: (left: number, right: number) => number): (left: number, right: number) => number {
            return callback;
        }
        function makeBinary(): (left: number, right: number) => number {
            console.log("immediate-target");
            return (left: number, right: number): number => left + right;
        }
        function immediateLeading(): [number] {
            console.log("immediate-leading");
            return [40];
        }
        function immediateTrailing(): number {
            console.log("immediate-trailing");
            return 2;
        }
        function main(): void {
            const callback = pass((value: number): number => value + 1);
            const args: [number] = [41];
            const leading: [number] = [40];
            const bound = passBinary((left: number, right: number): number => left + right)
                .bind((console.log("bind-this"), { marker: "bound" }), ...leading);
            console.log(callback.call(7, 41));
            console.log(callback.apply({ marker: "this" }, args));
            console.log(make().call((console.log("this"), 0), (console.log("argument"), 41)));
            console.log(bound(2));
            console.log(bound.call((console.log("rebound-this"), 0), 2));
            console.log(makeBinary()
                .bind((console.log("immediate-this"), null), ...immediateLeading())
                (immediateTrailing()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "function_call_apply_with_this"),
        "bind-this\n42\n42\ntarget\nthis\nargument\n42\n42\nrebound-this\n42\nimmediate-target\nimmediate-this\nimmediate-leading\nimmediate-trailing\n42\n"
    );
}

#[test]
fn compiles_unary_and_extended_comparisons_without_duplicate_evaluation() {
    let source = r#"
        function left(): number {
            console.log("left");
            return 2;
        }
        async function numberValue(): Promise<number> {
            await sleep(1);
            return 3;
        }
        async function boolValue(): Promise<boolean> {
            await sleep(1);
            return false;
        }
        async function main(): Promise<void> {
            console.log(-left());
            console.log(+2);
            console.log(!false);
            console.log(left() <= 2);
            console.log(2 >= 3);
            console.log("same" !== "same");
            console.log(-(await numberValue()));
            console.log(!(await boolValue()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "extended_operators"),
        "left\n-2\n2\ntrue\nleft\ntrue\nfalse\nfalse\n-3\ntrue\n"
    );
}

#[test]
fn compiles_remainder_exponentiation_and_compound_forms() {
    let source = r#"
        async function numberValue(): Promise<number> {
            await sleep(1);
            return 10;
        }
        async function main(): Promise<void> {
            let value = 10;
            console.log(value % 3);
            console.log(2 ** 3);
            value %= 4;
            value **= 3;
            console.log(value);
            console.log((await numberValue()) % 4);
            console.log(2 ** (await numberValue()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "remainder_exponentiation"),
        "1\n8\n8\n2\n1024\n"
    );
}

#[test]
fn compiles_bitwise_shift_and_compound_forms() {
    let source = r#"
        async function numberValue(): Promise<number> {
            await sleep(1);
            return 5;
        }
        async function main(): Promise<void> {
            console.log(5 | 2);
            console.log(5 ^ 1);
            console.log(5 & 3);
            console.log(3 << 2);
            console.log(-8 >> 2);
            console.log((-1 >>> 0) === 4294967295);
            console.log(1 << 33);
            let value = await numberValue();
            value |= 2;
            value ^= 1;
            value &= 6;
            value <<= 2;
            value >>= 1;
            value >>>= 1;
            console.log(value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "bitwise_shift"),
        "7\n4\n1\n12\n-2\ntrue\n2\n6\n"
    );
}

#[test]
fn compiles_prefix_and_postfix_updates_with_single_evaluation() {
    let source = r#"
        function index(): number {
            console.log("index");
            return 0;
        }
        async function asyncIndex(): Promise<number> {
            await sleep(1);
            console.log("async-index");
            return 0;
        }
        async function main(): Promise<void> {
            let value = 5;
            console.log(value++);
            console.log(value);
            console.log(++value);
            let values = [10];
            console.log(values[index()]--);
            console.log(values[0]);
            console.log(values[await asyncIndex()]++);
            console.log(values[0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "update_expression_values"),
        "5\n6\n7\nindex\n10\n9\nasync-index\n9\n10\n"
    );
}

#[test]
fn compound_assignments_evaluate_references_once_and_before_rhs() {
    let source = r#"
        function arraySource(values: number[]): number[] {
            console.log("array"); return values;
        }
        function index(): number { console.log("index"); return 0; }
        function rhs(): number { console.log("rhs"); return 5; }
        function objectSource(point: { value: number }): { value: number } {
            console.log("object"); return point;
        }
        async function asyncArray(values: number[]): Promise<number[]> {
            await sleep(1); console.log("async-array"); return values;
        }
        async function asyncIndex(): Promise<number> {
            await sleep(1); console.log("async-index"); return 0;
        }
        async function asyncRhs(): Promise<number> {
            await sleep(1); console.log("async-rhs"); return 2;
        }
        async function main(): Promise<void> {
            let values = [10];
            console.log(arraySource(values)[index()] += rhs());
            console.log(values[0]);
            let point = { value: 3 };
            console.log(objectSource(point).value *= rhs());
            console.log(point.value);
            console.log((await asyncArray(values))[await asyncIndex()] *= await asyncRhs());
            console.log(values[0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "compound_assignment_order"),
        "array\nindex\nrhs\n15\n15\nobject\nrhs\n15\n15\nasync-array\nasync-index\nasync-rhs\n30\n30\n"
    );
}

#[test]
fn logical_operators_preserve_native_operands_and_truthiness() {
    let source = r#"
        function numberValue(label: string, value: number): number {
            console.log(label);
            return value;
        }
        function stringValue(label: string, value: string): string {
            console.log(label);
            return value;
        }
        function arrayValue(): number[] {
            console.log("array-rhs");
            return [9];
        }
        function objectValue(): { value: number } {
            console.log("object-rhs");
            return { value: 9 };
        }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            console.log("awaited-rhs");
            return value;
        }
        async function main(): Promise<void> {
            console.log(0 || numberValue("number-rhs", 5));
            console.log(3 || numberValue("wrong-number-rhs", 8));
            console.log((0 / 0) || 7);
            console.log("" || stringValue("string-rhs", "fallback"));
            console.log("kept" || stringValue("wrong-string-rhs", "fallback"));
            const empty: number[] = [];
            console.log((empty || arrayValue()).length);
            const object: { value: number } = { value: 3 };
            console.log((object && objectValue()).value);
            console.log(0 && (await delayed(10)));
            console.log(1 && (await delayed(11)));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_logical_truthiness"),
        "number-rhs\n5\n3\n7\nstring-rhs\nfallback\nkept\n0\nobject-rhs\n9\n0\nawaited-rhs\n11\n"
    );
}

#[test]
fn compiles_native_boolean_conversion_with_single_evaluation() {
    let source = r#"
        function number(label: string, value: number): number {
            console.log(label);
            return value;
        }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            console.log("awaited-boolean");
            return value;
        }
        async function main(): Promise<void> {
            console.log(Boolean(number("zero", 0)));
            console.log(Boolean(number("nonzero", -2)));
            console.log(Boolean(0 / 0));
            console.log(Boolean(""));
            console.log(Boolean("text"));
            const values: number[] = [];
            console.log(Boolean(values));
            const object: { value: number } = { value: 0 };
            console.log(Boolean(object));
            console.log(Boolean(await delayed(0)));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_boolean_conversion"),
        "zero\nfalse\nnonzero\ntrue\nfalse\nfalse\ntrue\ntrue\ntrue\nawaited-boolean\nfalse\n"
    );
}

#[test]
fn compiles_primitive_relational_comparisons_with_utf16_order() {
    let source = r#"
        async function delayed(value: string): Promise<string> {
            await sleep(1);
            console.log("awaited-relation");
            return value;
        }
        async function main(): Promise<void> {
            console.log("apple" < "banana");
            console.log("same" <= "same");
            console.log("z" > "a");
            console.log("\u{10000}" < "\u{e000}");
            console.log("10" < 2);
            console.log(true >= "1");
            console.log((0 / 0) <= 1);
            console.log((await delayed("20")) > 3);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "primitive_relational"),
        "true\ntrue\ntrue\ntrue\nfalse\ntrue\nfalse\nawaited-relation\ntrue\n"
    );
}

#[test]
fn compiles_native_primitive_value_of() {
    let source = r#"
        function numberValue(): number {
            console.log("number");
            return 42;
        }
        function stringValue(): string {
            console.log("string");
            return "word";
        }
        function booleanValue(): boolean {
            console.log("boolean");
            return true;
        }
        async function delayedNumber(): Promise<number> {
            console.log("awaited number");
            await sleep(1);
            return 7;
        }
        async function delayedString(): Promise<string> {
            console.log("awaited string");
            await sleep(1);
            return "later";
        }
        async function delayedBoolean(): Promise<boolean> {
            console.log("awaited boolean");
            await sleep(1);
            return false;
        }
        async function main(): Promise<void> {
            console.log(numberValue().valueOf());
            console.log(stringValue().valueOf());
            console.log(booleanValue().valueOf());
            console.log((await delayedNumber()).valueOf());
            console.log((await delayedString()).valueOf());
            console.log((await delayedBoolean()).valueOf());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "primitive_value_of"),
        "number\n42\nstring\nword\nboolean\ntrue\nawaited number\n7\nawaited string\nlater\nawaited boolean\nfalse\n"
    );
}

#[test]
fn compiles_ecmascript_string_trimming() {
    let source = r#"
        function value(): string {
            console.log("receiver-evaluated");
            return "  thaw  ";
        }
        async function delayed(): Promise<string> {
            await sleep(1);
            console.log("awaited-trim");
            return "\u{feff}\u{00a0}done\u{3000}";
        }
        async function main(): Promise<void> {
            console.log(value().trim());
            console.log("  start  ".trimStart().startsWith("start"));
            console.log("  end  ".trimEnd().endsWith("end"));
            console.log("\u{0085}kept\u{0085}".trim().startsWith("\u{0085}"));
            console.log((await delayed()).trim());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_trim"),
        "receiver-evaluated\nthaw\ntrue\ntrue\ntrue\nawaited-trim\ndone\n"
    );
}

#[test]
fn compiles_unicode_string_case_conversion() {
    let source = r#"
        async function delayed(): Promise<string> {
            console.log("awaited");
            await sleep(1);
            return "Straße";
        }
        async function main(): Promise<void> {
            console.log("ThAw".toLowerCase());
            console.log("ThAw".toUpperCase());
            console.log("Straße".toUpperCase());
            console.log("İ".toLowerCase());
            console.log("".toUpperCase().length);
            console.log((await delayed()).toUpperCase());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_case_conversion"),
        "thaw\nTHAW\nSTRASSE\ni̇\n0\nawaited\nSTRASSE\n"
    );
}

#[test]
fn compares_builtin_string_results_by_contents() {
    let source = r#"
        function main(): void {
            console.log(" x ".trim() === "x");
            console.log(" x ".trim() !== "x");
            console.log(String(42) === "42");
            console.log(("a" + "b") === "ab");
            const values: number[] = [1, 2];
            console.log(values.join("-") === "1-2");
            console.log(values.toString() === "1,2");
            console.log(JSON.stringify(JSON.parse("{\"x\":1}")) === "{\"x\":1}");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "builtin_string_equality"),
        "true\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\n"
    );
}

#[test]
fn compiles_utf16_string_length_and_char_code_at() {
    let source = r#"
        function text(): string {
            console.log("receiver");
            return "abc";
        }
        function index(): string {
            console.log("index");
            return "2";
        }
        async function delayed(): Promise<string> {
            await sleep(1);
            console.log("awaited-string");
            return "😀";
        }
        async function delayedIndex(): Promise<number> {
            console.log("awaited-index");
            await sleep(1);
            return 1;
        }
        async function main(): Promise<void> {
            const unicode: string = "😀a";
            console.log(unicode.length);
            console.log(unicode.charCodeAt(0));
            console.log(unicode.charCodeAt(1));
            console.log(unicode.charCodeAt(2));
            console.log("a".charCodeAt());
            console.log("a".charCodeAt(0 / 0));
            console.log(Number.isNaN(unicode.charCodeAt(-1)));
            console.log(Number.isNaN(unicode.charCodeAt(Infinity)));
            console.log(text().charCodeAt(index()));
            console.log(text().charCodeAt(await delayedIndex()));
            console.log((await delayed()).length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_utf16_access"),
        "3\n55357\n56832\n97\n97\n97\ntrue\ntrue\nreceiver\nindex\n99\nreceiver\nawaited-index\n98\nawaited-string\n2\n"
    );
}

#[test]
fn compiles_unary_math_functions_with_native_coercion() {
    let source = r#"
        function text(value: string): string {
            console.log("math-argument");
            return value;
        }
        async function delayed(value: string): Promise<string> {
            await sleep(1);
            console.log("awaited-math");
            return value;
        }
        async function main(): Promise<void> {
            console.log(Math.abs(text("-3.5")));
            console.log(Math.floor(2.9));
            console.log(Math.ceil(-2.9));
            console.log(Math.trunc(-2.9));
            console.log(Math.sqrt(true));
            const one: number[] = [9];
            console.log(Math.sqrt(one));
            console.log(Number.isNaN(Math.sqrt(-1)));
            console.log((1 / Math.trunc(-0.5)) < 0);
            console.log(Math.floor(await delayed("4.8")));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "unary_math_functions"),
        "math-argument\n3.5\n2\n-2\n-2\n1\n3\ntrue\ntrue\nawaited-math\n4\n"
    );
}

#[test]
fn compiles_integer_and_safe_integer_predicates() {
    let source = r#"
        function text(): string {
            console.log("integer-non-number");
            return "1";
        }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            console.log("awaited-integer");
            return value;
        }
        async function main(): Promise<void> {
            console.log(Number.isInteger(3));
            console.log(Number.isInteger(3.5));
            console.log(Number.isInteger(0 / 0));
            console.log(Number.isInteger(Number("Infinity")));
            console.log(Number.isInteger(-0));
            console.log(Number.isInteger(text()));
            console.log(Number.isSafeInteger(9007199254740991));
            console.log(Number.isSafeInteger(9007199254740992));
            console.log(Number.isSafeInteger(-9007199254740991));
            console.log(Number.isSafeInteger(-9007199254740992));
            console.log(Number.isSafeInteger(await delayed(42)));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "integer_predicates"),
        "true\nfalse\nfalse\nfalse\ntrue\ninteger-non-number\nfalse\ntrue\nfalse\ntrue\nfalse\nawaited-integer\ntrue\n"
    );
}

#[test]
fn compiles_pow_extrema_and_sign_math_functions() {
    let source = r#"
        function number(label: string, value: number): number {
            console.log(label);
            return value;
        }
        async function delayed(value: string): Promise<string> {
            await sleep(1);
            console.log("awaited-extreme");
            return value;
        }
        async function main(): Promise<void> {
            console.log(Math.pow("2", "3"));
            console.log(Math.min(number("min-left", 4), number("min-right", -2), 7));
            console.log(Math.max(4, -2, 7));
            console.log(Number.isNaN(Math.min(1, 0 / 0, 2)));
            console.log(Number.isFinite(Math.min()));
            console.log(Math.min() > 0);
            console.log(Number.isFinite(Math.max()));
            console.log(Math.max() < 0);
            console.log((1 / Math.min(0, -0)) < 0);
            console.log((1 / Math.max(-0, 0)) > 0);
            console.log(Math.sign(-8));
            console.log(Math.sign(9));
            console.log((1 / Math.sign(-0)) < 0);
            console.log(Number.isNaN(Math.sign(0 / 0)));
            console.log(Math.max(1, await delayed("12"), 3));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "math_pow_extrema_sign"),
        "8\nmin-left\nmin-right\n-2\n7\ntrue\nfalse\ntrue\nfalse\ntrue\ntrue\ntrue\n-1\n1\ntrue\ntrue\nawaited-extreme\n12\n"
    );
}

#[test]
fn compiles_javascript_math_round_semantics() {
    let source = r#"
        async function delayed(): Promise<string> {
            await sleep(1);
            console.log("awaited-round");
            return "4.6";
        }
        async function main(): Promise<void> {
            console.log(Math.round(1.4));
            console.log(Math.round(1.5));
            console.log(Math.round(-1.5));
            console.log(Math.round(-1.6));
            console.log((1 / Math.round(-0.1)) < 0);
            console.log((1 / Math.round(-0.5)) < 0);
            console.log((1 / Math.round(-0)) < 0);
            console.log(Number.isNaN(Math.round(0 / 0)));
            console.log(Number.isFinite(Math.round(Number("Infinity"))));
            console.log(Math.round(await delayed()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "math_round"),
        "1\n2\n-1\n-2\ntrue\ntrue\ntrue\ntrue\nfalse\nawaited-round\n5\n"
    );
}

#[test]
fn compiles_transcendental_math_intrinsics() {
    let source = r#"
        function value(): string {
            console.log("value-evaluated");
            return "8";
        }
        async function delayed(): Promise<string> {
            await sleep(1);
            console.log("awaited-math");
            return "1000";
        }
        async function main(): Promise<void> {
            console.log(Math.exp(0) === 1);
            console.log(Math.log(1) === 0);
            console.log(Math.log2(value()) === 3);
            console.log(Math.log10(await delayed()) === 3);
            console.log(Math.sin(0) === 0);
            console.log(Math.cos(0) === 1);
            console.log(Number.isFinite(Math.log(0)));
            console.log(Math.log(0) < 0);
            console.log(Number.isNaN(Math.log(-1)));
            console.log(Number.isFinite(Math.exp(Number("Infinity"))));
            console.log(Number.isNaN(Math.sin(Number("Infinity"))));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "transcendental_math"),
        "true\ntrue\nvalue-evaluated\ntrue\nawaited-math\ntrue\ntrue\ntrue\nfalse\ntrue\ntrue\nfalse\ntrue\n"
    );
}

#[test]
fn compiles_extended_libm_functions() {
    let source = r#"
        function value(label: string, result: string): string {
            console.log(label);
            return result;
        }
        async function delayed(): Promise<string> {
            await sleep(1);
            console.log("awaited-hypot");
            return "12";
        }
        async function main(): Promise<void> {
            console.log(Math.tan(0) === 0);
            console.log(Math.asin(0) === 0);
            console.log(Math.acos(1) === 0);
            console.log(Math.atan(0) === 0);
            console.log(Math.sinh(0) === 0);
            console.log(Math.cosh(0) === 1);
            console.log(Math.tanh(0) === 0);
            console.log(Math.cbrt(-8) === -2);
            console.log((1 / Math.atan2(-0, 1)) < 0);
            console.log(Math.atan2(0, -1) > 3);
            console.log(Math.hypot() === 0);
            console.log(Math.hypot(value("left", "3"), value("right", "4")) === 5);
            console.log(Math.hypot("6", "8") === 10);
            console.log(Number.isFinite(Math.hypot(Number("Infinity"), 0 / 0)));
            console.log(Math.hypot(5, await delayed()) === 13);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "extended_libm"),
        "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nleft\nright\ntrue\ntrue\nfalse\nawaited-hypot\ntrue\n"
    );
}

#[test]
fn compiles_near_zero_libm_functions() {
    let source = r#"
        async function delayed(): Promise<string> {
            await sleep(1);
            console.log("awaited-log1p");
            return "0";
        }
        async function main(): Promise<void> {
            console.log(Math.acosh(1) === 0);
            console.log(Number.isNaN(Math.acosh(0)));
            console.log((1 / Math.asinh(-0)) < 0);
            console.log(Math.atanh(0) === 0);
            console.log(Number.isFinite(Math.atanh(1)));
            console.log(Math.atanh(1) > 0);
            console.log((1 / Math.expm1(-0)) < 0);
            console.log((1 / Math.log1p(-0)) < 0);
            console.log(Number.isNaN(Math.log1p(-2)));
            console.log(Math.log1p(await delayed()) === 0);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "near_zero_libm"),
        "true\ntrue\ntrue\ntrue\nfalse\ntrue\ntrue\ntrue\ntrue\nawaited-log1p\ntrue\n"
    );
}

#[test]
fn compiles_integer_and_single_precision_math_functions() {
    let source = r#"
        function value(label: string, result: string): string {
            console.log(label);
            return result;
        }
        async function delayed(): Promise<string> {
            await sleep(1);
            console.log("awaited-clz32");
            return "16";
        }
        async function main(): Promise<void> {
            const rounded: number = Math.fround(1.337);
            console.log(rounded !== 1.337);
            console.log(Math.fround(rounded) === rounded);
            console.log((1 / Math.fround(-0)) < 0);
            console.log(Number.isFinite(Math.fround(Number("Infinity"))));
            console.log(Math.clz32(0));
            console.log(Math.clz32(1));
            console.log(Math.clz32(-1));
            console.log(Math.clz32(15));
            console.log(Math.clz32(0 / 0));
            console.log(Math.imul(-1, 5));
            console.log(Math.imul(2147483647, 2));
            console.log(Math.imul(true, 7.9));
            console.log(Math.imul(value("left", "3"), value("right", "4")));
            console.log(Math.clz32(await delayed()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "integer_single_precision_math"),
        "true\ntrue\ntrue\nfalse\n32\n31\n0\n28\n32\n-5\n-2\n7\nleft\nright\n12\nawaited-clz32\n27\n"
    );
}

#[test]
fn compiles_stateful_math_random() {
    let source = r#"
        function main(): void {
            const first: number = Math.random();
            const second: number = Math.random();
            console.log(first >= 0);
            console.log(first < 1);
            console.log(second >= 0);
            console.log(second < 1);
            console.log(first !== second);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "math_random"),
        "true\ntrue\ntrue\ntrue\ntrue\n"
    );
}

#[test]
fn compiles_standard_math_constants() {
    let source = r#"
        function main(): void {
            console.log(Math.E === 2.718281828459045);
            console.log(Math.PI === 3.141592653589793);
            console.log(Math.LN2 === 0.6931471805599453);
            console.log(Math.LN10 === 2.302585092994046);
            console.log(Math.LOG2E === 1.4426950408889634);
            console.log(Math.LOG10E === 0.4342944819032518);
            console.log(Math.SQRT1_2 === 0.7071067811865476);
            console.log(Math.SQRT2 === 1.4142135623730951);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "math_constants"),
        "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"
    );
}

#[test]
fn compiles_standard_number_constants() {
    let source = r#"
        function main(): void {
            console.log(Number.isNaN(Number.NaN));
            console.log(Number.isFinite(Number.POSITIVE_INFINITY));
            console.log(Number.NEGATIVE_INFINITY < 0);
            console.log(Number.MAX_VALUE === 1.7976931348623157e308);
            console.log(Number.MIN_VALUE > 0);
            console.log(Number.MIN_VALUE / 2 === 0);
            console.log(Number.MAX_SAFE_INTEGER === 9007199254740991);
            console.log(Number.MIN_SAFE_INTEGER === -9007199254740991);
            console.log(1 + Number.EPSILON > 1);
            console.log(1 + Number.EPSILON / 2 === 1);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "number_constants"),
        "true\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"
    );
}

#[test]
fn compiles_global_nan_and_infinity() {
    let source = r#"
        function shadow(NaN: number, Infinity: number): number {
            return NaN + Infinity;
        }
        function main(): void {
            console.log(Number.isNaN(NaN));
            console.log(Number.isFinite(Infinity));
            console.log(-Infinity < 0);
            console.log(shadow(20, 22));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "global_non_finite_numbers"),
        "true\nfalse\ntrue\n42\n"
    );
}

#[test]
fn compiles_parse_float_and_parse_int() {
    let source = r#"
        async function delayed(): Promise<string> {
            await sleep(1);
            console.log("awaited-parse");
            return "101tail";
        }
        function parseArgs(): [string, number] {
            console.log("parse-spread");
            return ["ff", 16];
        }
        async function main(): Promise<void> {
            console.log(parseFloat("  -12.5px"));
            console.log(parseFloat("1e+"));
            console.log(parseFloat(true));
            console.log(parseInt("0x20"));
            console.log(parseInt("11", 2));
            console.log(parseInt("15px", "10"));
            console.log(Number.isNaN(parseInt("10", 1)));
            console.log((1 / parseInt("-0", 10)) < 0);
            console.log(parseInt(await delayed(), 2));
            console.log(Number.parseFloat("3.5tail"));
            console.log(Number.parseInt("ff", 16));
            console.log(parseFloat(...["2.5tail"]));
            console.log(Number.parseInt(...parseArgs()));
            console.log(isNaN(...["not-number"]));
            console.log(isFinite(...["12"]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "parse_float_int"),
        "-12.5\n1\nnan\n32\n3\n15\ntrue\ntrue\nawaited-parse\n5\n3.5\n255\n2.5\nparse-spread\n255\ntrue\ntrue\n"
    );
}

#[test]
fn compiles_tuple_spreads_for_math_and_number_builtins() {
    let source = r#"
        function numeric(): [string] {
            console.log("numeric-spread");
            return ["9"];
        }
        function main(): void {
            console.log(Math.sqrt(...numeric()));
            console.log(Math.pow(...[2, 3]));
            console.log(Math.min(...[3, 1, 2]));
            console.log(Math.hypot(...[3, 4]));
            console.log(Number.isInteger(...[3]));
            console.log(Number.isNaN(...["not-number"]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "math_number_tuple_spreads"),
        "numeric-spread\n3\n8\n1\n5\ntrue\nfalse\n"
    );
}

#[test]
fn compiles_tuple_spreads_for_string_and_search_builtins() {
    let source = r#"
        function main(): void {
            console.log("ABC".charCodeAt(...[1]));
            console.log("x".concat(...["y", 1]));
            console.log([1].concat(...[[2, 3], [4]]).join(","));
            console.log("ab".repeat(...[2]));
            console.log("hello".indexOf(...["l", 3]));
            console.log("hello".includes(...["ell", 0]));
            console.log([1, 2, 3].lastIndexOf(...[2, 2]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_search_tuple_spreads"),
        "66\nxy1\n1,2,3,4\nabab\n3\ntrue\n1\n"
    );
}

#[test]
fn compiles_plain_function_rest_parameters() {
    let source = r#"
        function sum(...nums: number[]): number {
            let total = 0;
            for (const n of nums) {
                total += n;
            }
            return total;
        }
        function label(prefix: string, ...parts: string[]): string {
            return prefix + ":" + parts.join(",");
        }
        function main(): void {
            console.log(sum());
            console.log(sum(1));
            console.log(sum(1, 2, 3));
            console.log(sum(...[4, 5, 6]));
            console.log(label("x"));
            console.log(label("x", "a", "b"));
            console.log(label("x", ...["p", "q"]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "plain_function_rest_parameters"),
        "0\n1\n6\n15\nx:\nx:a,b\nx:p,q\n"
    );
}

#[test]
fn nested_finally_blocks_run_inside_out() {
    let source = r#"
        function nested(): string {
            try {
                try {
                    return "done";
                } finally {
                    console.log("inner");
                }
            } finally {
                console.log("outer");
            }
        }

        function main(): void {
            console.log(nested());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nested_finally"),
        "inner\nouter\ndone\n"
    );
}

/// The QuickJS-NG fallback path (docs/design/bridge.md section 7): a
/// compiled Thaw program loads real JS source and calls into it,
/// round-tripping arguments/results through `Json`.
#[test]
fn compiles_quickjs_fallback_path() {
    let source = r#"
        function main(): void {
            const ok: boolean = loadScript(
                "function add(a, b) { return a + b; } function greet(name) { return 'hello, ' + name; } function later(x) { return Promise.resolve(x * 2); } function foreignThenable() { return { then(resolve, reject) { resolve(84); reject('late'); resolve(99); } }; } globalThis.retainedMultiplier = (factor => value => Promise.resolve(value * factor))(2); globalThis.limiterFactory = count => task => Promise.resolve(task()); globalThis.retainedTask = () => 42; globalThis.dynamicBox = { value: 1, add(n) { this.value += n; return this.value; } }; globalThis.dynamicReplacement = 9; globalThis.throwingBox = Object.create(null, { bad: { get() { throw new Error('getter failed'); } } }); globalThis.mixedCall = (base, label, first, second) => label + ':' + (base + first() + second()); globalThis.DynamicBox = class { constructor(value) { this.value = value; } }; globalThis.dynamicUndefined = undefined;"
            );
            console.log(ok);

            const sum = callDynamic("add", JSON.parse("[2, 3]"));
            console.log(Number(sum));

            const greeting = callDynamic("greet", JSON.parse("[\"thaw\"]"));
            console.log(String(greeting));

            const doubled = callDynamic("later", JSON.parse("[21]"));
            console.log(Number(doubled));

            const assimilated = callDynamic("foreignThenable", JSON.parse("[]"));
            console.log(Number(assimilated));

            const callable: JsValue = getDynamicValue("retainedMultiplier");
            const called = callDynamicValue(callable, JSON.parse("[21]"));
            console.log(Number(called));

            const factory: JsValue = getDynamicValue("limiterFactory");
            const limiter: JsValue = callDynamicValueHandle(factory, JSON.parse("[2]"));
            const task: JsValue = getDynamicValue("retainedTask");
            const limited = callDynamicValueWithValue(limiter, task);
            console.log(Number(limited));
            console.log(releaseDynamicValue(limiter));
            console.log(releaseDynamicValue(limiter));

            const box: JsValue = getDynamicValue("dynamicBox");
            console.log(Number(callDynamicMethod(box, "add", JSON.parse("[2]"))));
            const property: JsValue = getDynamicProperty(box, "value");
            console.log(Number(readDynamicValue(property)));
            const replacement: JsValue = getDynamicValue("dynamicReplacement");
            console.log(setDynamicProperty(box, "value", replacement));
            console.log(Number(callDynamicMethod(box, "add", JSON.parse("[1]"))));
            const throwing: JsValue = getDynamicValue("throwingBox");
            try {
                const bad: JsValue = getDynamicProperty(throwing, "bad");
            } catch (error) {
                console.log(error);
            }
            const mixed: JsValue = getDynamicValue("mixedCall");
            console.log(String(callDynamicValueMixed(mixed, JSON.parse("[2, \"sum\"]"), [task, task])));
            const boxConstructor: JsValue = getDynamicValue("DynamicBox");
            const constructed: JsValue = constructDynamicValue(boxConstructor, JSON.parse("[42]"));
            console.log(Number(readDynamicValue(getDynamicProperty(constructed, "value"))));
            const undefinedValue: JsValue = getDynamicValue("dynamicUndefined");
            console.log(releaseDynamicValue(undefinedValue));
            const symbolFactory: JsValue = getDynamicValue("Symbol");
            const symbolValue: JsValue = callDynamicValueHandle(symbolFactory, JSON.parse("[\"token\"]"));
            console.log(releaseDynamicValue(symbolValue));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "quickjs_fallback"),
        "true\n5\nhello, thaw\n42\n84\n42\n42\ntrue\nfalse\n3\n3\ntrue\n10\ngetter failed\nsum:86\n42\ntrue\ntrue\n"
    );
}

/// Module auto-initialization: a registry-generated `__thaw_module_init`
/// (thaw-bridge's `generate_module_init`, wired in via thaw-cli's
/// `--use`) must run before `main`'s body, with no `loadScript` call
/// written by the user -- that's the whole point of automating it.
#[test]
fn runs_module_init_before_main_body() {
    let source = r#"
        function __thaw_module_init(): void {
            loadScript("function greet() { return 'hi from registry'; }");
        }

        function main(): void {
            const result = callDynamic("greet", JSON.parse("[]"));
            console.log(String(result));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "module_init_main"),
        "hi from registry\n"
    );
}

#[test]
fn initializes_top_level_bindings_in_source_order_and_shares_mutation() {
    let source = r#"
        const base = 40;
        let answer = base + 2;

        function next(): number {
            answer = answer + 1;
            return answer;
        }

        function main(): void {
            console.log(answer);
            console.log(next());
            console.log(next());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "top_level_bindings"),
        "42\n43\n44\n"
    );
}

#[test]
fn executes_top_level_statements_before_main() {
    let source = r#"
        let answer = 40;
        console.log("module init");
        answer = answer + 1;
        if (true) { answer++; }
        function main(): void { console.log(answer); }
    "#;
    assert_eq!(
        compile_and_run(source, "top_level_statements"),
        "module init\n42\n"
    );
}

#[test]
fn compiles_and_runs_inherited_members_and_overrides() {
    let source = r#"
        class Base {
            constructor(public value: number) {}
            answer(): number { return this.value; }
            get doubled(): number { return this.value * 2; }
            set current(next: number) { this.value = next; }
        }
        class Derived extends Base {
            constructor(value: number) { super(value); }
            answer(): number { return this.value + 1; }
        }
        function main(): void {
            const value = new Derived(20);
            value.current = 21;
            console.log(value.doubled);
            console.log(value.answer());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "inherited_members_and_overrides"),
        "42\n22\n"
    );
}

#[test]
fn compiles_empty_and_debugger_statements_as_no_ops() {
    let source = r#"
        function main(): void {
            ;;;
            debugger;
            let value: number = 0;
            while (value < 2) {
                ;
                value = value + 1;
                debugger;
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "empty_debugger_statements"), "2\n");
}

#[test]
fn reads_process_env() {
    let source = r#"
        function main(): void {
            console.log(process.env.THAW_TEST_VAR);
        }
    "#;
    assert_eq!(
        compile_and_run_with_env(source, "envvar", &[("THAW_TEST_VAR", "hello-env")]),
        "hello-env\n"
    );
}

#[test]
fn unset_env_var_reads_as_empty_string_not_a_crash() {
    let source = r#"
        function main(): void {
            console.log(process.env.THAW_DEFINITELY_UNSET_VAR_XYZ);
            console.log("still alive");
        }
    "#;
    assert_eq!(compile_and_run(source, "envvar_unset"), "\nstill alive\n");
}
