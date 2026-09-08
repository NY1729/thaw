#[test]
fn calls_optional_function_values_across_typed_boundaries() {
    let source = r#"
        interface Holder {
            run: (prefix: string, value?: number) => string;
        }
        function pass(
            callback: (prefix: string, value?: number) => string
        ): (prefix: string, value?: number) => string {
            return callback;
        }
        function namedFormat(prefix: string, value: number = 6): string {
            return prefix + String(value);
        }
        function main(): void {
            const format: (prefix: string, value?: number) => string =
                (prefix: string, value: number = 5): string =>
                    prefix + String(value);
            const through = pass(format);
            const named: (prefix: string, value?: number) => string = namedFormat;
            const passedNamed = pass(namedFormat);
            const maybe: ((prefix: string, value?: number) => string) | undefined = through;
            const holder: Holder = { run: through };
            const one: [string] = ["apply="];
            const bound = through.bind(null, "bound=");
            console.log(through("direct="));
            console.log(through("value=", 7));
            console.log(holder.run("object="));
            console.log(through.call(null, "call="));
            console.log(through.apply(null, one));
            console.log(bound());
            console.log(bound(8));
            console.log(through.bind(null, "immediate=")());
            console.log(named("named="));
            console.log(passedNamed("passed="));
            console.log(maybe?.("chain="));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_function_boundary"),
        "direct=5\nvalue=7\nobject=5\ncall=5\napply=5\nbound=5\nbound=8\nimmediate=5\nnamed=6\npassed=6\nchain=5\n"
    );
}

#[test]
fn combines_optional_and_rest_function_value_abis() {
    let source = r#"
        function main(): void {
            const format: (
                prefix: string,
                separator?: string,
                ...values: string[]
            ) => string = (
                prefix: string,
                separator: string = "|",
                ...values: string[]
            ): string => prefix + values.join(separator);
            const bound = format.bind(null, "bound:");
            console.log(format("empty:"));
            console.log(format("values:", undefined, "a", "b"));
            console.log(format.call(null, "call:", ",", "x", "y"));
            console.log(bound());
            console.log(bound("-", "c", "d"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_rest_function_boundary"),
        "empty:\nvalues:a|b\ncall:x,y\nbound:\nbound:c-d\n"
    );
}

#[test]
fn calls_a_closure_stored_in_an_object_property() {
    let source = r#"
        interface Operations { apply: (value: number) => number; }
        function main(): void {
            const offset: number = 40;
            const operations: Operations = {
                apply: (value: number): number => offset + value
            };
            console.log(operations.apply(2));
        }
    "#;
    assert_eq!(compile_and_run(source, "object_method_closure"), "42\n");
}

#[test]
fn compiles_while_loop_with_array_mutation() {
    let source = r#"
        function main(): void {
            const xs: number[] = [0, 0, 0];
            let i: number = 0;
            while (i < 3) {
                xs[i] = i * i;
                i = i + 1;
            }
            console.log(xs[0]);
            console.log(xs[1]);
            console.log(xs[2]);
        }
    "#;
    assert_eq!(compile_and_run(source, "whileloop"), "0\n1\n4\n");
}

#[test]
fn compiles_array_spreads_for_all_native_element_shapes() {
    let source = r#"
        interface Item { value: number; }
        function first(): number[] {
            console.log("first");
            return [2, 3];
        }
        function second(): number[] {
            console.log("second");
            return [5];
        }
        function main(): void {
            const middle: number[] = [4];
            const numbers: number[] = [1, ...first(), ...middle, ...second(), 6];
            console.log(numbers.length);
            console.log(numbers[0]);
            console.log(numbers[5]);

            const strings: string[] = ["a", ...["b", "c"]];
            console.log(strings[1]);
            const booleans: boolean[] = [true, ...[false]];
            console.log(booleans[1]);
            const extra: Item[] = [{ value: 2 }];
            const objects: Item[] = [{ value: 1 }, ...extra];
            console.log(objects[1].value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_spreads"),
        "first\nsecond\n6\n1\n6\nb\nfalse\n2\n"
    );
}

#[test]
fn compiles_string_spreads_into_array_literals() {
    let source = r#"
        function main(): void {
            printAll(["x", ..."ab", "y"]);
            printAll([..."abc", ..."def"]);
            printAll([...""]);
            console.log([...""].length);
        }
        function printAll(parts: string[]): void {
            for (const part of parts) {
                console.log(part);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_spreads_into_array_literals"),
        "x\na\nb\ny\na\nb\nc\nd\ne\nf\n0\n"
    );
}

#[test]
fn compiles_map_and_set_spreads_into_array_literals() {
    let source = r#"
        function main(): void {
            const map = new Map<string, number>();
            map.set("a", 1).set("b", 2);
            const pairs = [...map];
            console.log(pairs.length);
            console.log(pairs[0][0], pairs[0][1]);
            console.log(pairs[1][0], pairs[1][1]);

            const set = new Set<number>([1, 2, 3]);
            const values = [0, ...set, 4];
            console.log(values.length);
            for (const value of values) {
                console.log(value);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "map_set_spreads_into_array_literals"),
        "2\na 1\nb 2\n5\n0\n1\n2\n3\n4\n"
    );
}

#[test]
fn strict_equality_supports_strings_booleans_and_object_identity() {
    let source = r#"
        interface Box { value: number; }
        function label(): string { return "same"; }
        function main(): void {
            const text = "same";
            console.log(text === label());
            console.log(true === false);
            const box: Box = { value: 1 };
            const alias: Box = box;
            console.log(box === alias);
            console.log(box === { value: 1 });
        }
    "#;
    assert_eq!(
        compile_and_run(source, "typed_strict_equality"),
        "true\nfalse\ntrue\nfalse\n"
    );
}

#[test]
fn compiles_call_argument_spreads_in_left_to_right_order() {
    let source = r#"
        function first(): number {
            console.log("first");
            return 1;
        }
        async function middle(): Promise<[string, number]> {
            await sleep(1);
            console.log("middle");
            return ["two", 3];
        }
        function last(): boolean {
            console.log("last");
            return true;
        }
        function emit(a: number, b: string, c: number, d: boolean): void {
            console.log(a);
            console.log(b);
            console.log(c);
            console.log(d);
        }
        async function main(): Promise<void> {
            emit(first(), ...(await middle()), last());
            emit(...[4, "five", 6, false]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "call_argument_spread"),
        "first\nmiddle\nlast\n1\ntwo\n3\ntrue\n4\nfive\n6\nfalse\n"
    );
}

