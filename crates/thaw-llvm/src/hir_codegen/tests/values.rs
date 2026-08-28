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

#[test]
fn compiles_native_null_literals() {
    let source = r#"
        function getNull(): null {
            console.log("get null");
            return null;
        }
        function main(): void {
            const value: null = null;
            console.log(value);
            console.log(typeof value);
            console.log(null === null);
            console.log(null === undefined);
            console.log(null !== undefined);
            console.log(getNull() == undefined);
            console.log(getNull() != undefined);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_null"),
        "null\nobject\ntrue\nfalse\ntrue\nget null\ntrue\nget null\nfalse\n"
    );
}

#[test]
fn compiles_native_nullable_values() {
    let source = r#"
        interface Box { value: number | null; label: string; }
        interface Item {
            value: number;
            maybe: number | null;
            run: (value: number) => number;
            runMaybe: (present: boolean) => number | null;
        }
        function maybe(present: boolean): number | null {
            if (present) return 4;
            return null;
        }
        function fallback(): number {
            console.log("fallback");
            return 9;
        }
        function maybeItem(present: boolean): Item | null {
            if (present) return {
                value: 5,
                maybe: null,
                run: (value: number) => value * 2,
                runMaybe: (present: boolean) => maybe(present)
            };
            return null;
        }
        function maybeText(present: boolean): string | null {
            if (present) return "thaw";
            return null;
        }
        function maybeCallback(present: boolean): ((value: number) => number) | null {
            if (present) return (value: number) => value + 1;
            return null;
        }
        function maybeNullableCallback(present: boolean): (() => number | null) | null {
            if (present) return () => maybe(false);
            return null;
        }
        function narrowed(value: number | null): number {
            if (value !== null) return value + 2;
            return 0;
        }
        function guarded(value: number | null): number {
            if (value === null) return 1;
            return value * 2;
        }
        async function delayed(present: boolean): Promise<string | null> {
            await sleep(1);
            if (present) return "ok";
            return null;
        }
        async function main(): Promise<void> {
            console.log(maybe(true));
            console.log(maybe(false));
            console.log(maybe(true) ?? fallback());
            console.log(maybe(false) ?? fallback());
            console.log(maybe(false) === null);
            console.log(maybe(true) === null);
            console.log(maybe(false) == undefined);
            console.log(maybe(true) == undefined);
            console.log(typeof maybe(true));
            console.log(typeof maybe(false));
            let value: number | null = null;
            console.log(value);
            value = 6;
            console.log(value);
            value = null;
            console.log(value ??= fallback());
            console.log(value ??= fallback());
            const box: Box = { value: null, label: "box" };
            console.log(box.value);
            console.log(box.label);
            console.log(maybeItem(true)?.value);
            console.log(maybeItem(false)?.value);
            console.log(maybeItem(true)?.maybe);
            console.log(maybeItem(false)?.maybe);
            console.log(maybeItem(true)?.run(3));
            console.log(maybeItem(false)?.run(fallback()));
            console.log(maybeItem(true)?.["runMaybe"](false));
            console.log(maybeItem(false)?.["runMaybe"](true));
            console.log(maybeText(true)?.length);
            console.log(maybeText(false)?.toUpperCase());
            console.log(maybeCallback(true)?.(7));
            console.log(maybeCallback(false)?.(fallback()));
            console.log(maybeNullableCallback(true)?.());
            console.log(maybeNullableCallback(false)?.());
            console.log(narrowed(3));
            console.log(narrowed(null));
            console.log(guarded(4));
            console.log(guarded(null));
            console.log(await delayed(true));
            console.log(await delayed(false));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_nullable"),
        "4\nnull\n4\nfallback\n9\ntrue\nfalse\ntrue\nfalse\nnumber\nobject\nnull\n6\nfallback\n9\n9\nnull\nbox\n5\nundefined\nnull\nundefined\n6\nundefined\nnull\nundefined\n4\nundefined\n8\nundefined\nnull\nundefined\n5\n0\n8\n1\nok\nnull\n"
    );
}

#[test]
fn compiles_native_three_way_nullish_values() {
    let source = r#"
        interface Box { value: number | null | undefined; label: string; }
        interface Item { value: number; }
        function maybe(kind: number): number | null | undefined {
            if (kind === 1) return 4;
            if (kind === 2) return null;
            return undefined;
        }
        function show(value: number | null | undefined): void {
            console.log(value);
        }
        function maybeItem(kind: number): Item | null | undefined {
            if (kind === 1) return { value: 6 };
            if (kind === 2) return null;
            return undefined;
        }
        function narrowed(value: number | null | undefined): number {
            if (value != null) return value + 1;
            return 0;
        }
        function guarded(value: number | null | undefined): number {
            if (value == null) return 0;
            return value * 2;
        }
        async function delayed(kind: number): Promise<number | null | undefined> {
            await sleep(1);
            return maybe(kind);
        }
        async function main(): Promise<void> {
            show(maybe(1));
            show(maybe(2));
            show(maybe(3));
            console.log(maybe(1) ?? 9);
            console.log(maybe(2) ?? 9);
            console.log(maybe(3) ?? 9);
            console.log(maybe(2) === null);
            console.log(maybe(3) === null);
            console.log(maybe(3) === undefined);
            console.log(maybe(2) === undefined);
            console.log(maybe(2) == undefined);
            console.log(maybe(3) == null);
            console.log(typeof maybe(1));
            console.log(typeof maybe(2));
            console.log(typeof maybe(3));
            console.log(maybeItem(1)?.value);
            console.log(maybeItem(2)?.value);
            console.log(maybeItem(3)?.value);
            let value: number | null | undefined = null;
            console.log(value);
            value = 8;
            console.log(value);
            value = undefined;
            console.log(value);
            console.log(value ??= 11);
            console.log(value ??= 12);
            console.log(value + 1);
            console.log(narrowed(maybe(1)));
            console.log(narrowed(maybe(2)));
            console.log(narrowed(maybe(3)));
            console.log(guarded(maybe(1)));
            console.log(guarded(maybe(2)));
            console.log(guarded(maybe(3)));
            const box: Box = { value: null, label: "box" };
            console.log(box.value);
            console.log(box.label);
            console.log(await delayed(1));
            console.log(await delayed(2));
            console.log(await delayed(3));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_three_way_nullish"),
        "4\nnull\nundefined\n4\n9\n9\ntrue\nfalse\ntrue\nfalse\ntrue\ntrue\nnumber\nobject\nundefined\n6\nundefined\nundefined\nnull\n8\nundefined\n11\n11\n12\n5\n0\n0\n8\n0\n0\nnull\nbox\n4\nnull\nundefined\n"
    );
}

#[test]
fn compiles_arrays_of_tagged_nullish_values() {
    let source = r#"
        function optional(present: boolean): number | undefined {
            if (present) return 1;
            return undefined;
        }
        function nullable(present: boolean): number | null {
            if (present) return 2;
            return null;
        }
        function nullish(kind: number): number | null | undefined {
            if (kind === 1) return 3;
            if (kind === 2) return null;
            return undefined;
        }
        function main(): void {
            const optionals: (number | undefined)[] = [
                optional(true), optional(false), optional(true)
            ];
            console.log(optionals[0]);
            console.log(optionals[1]);
            console.log(optionals[2]);
            const nullables: (number | null)[] = [
                nullable(true), nullable(false), nullable(true)
            ];
            console.log(nullables[0]);
            console.log(nullables[1]);
            console.log(nullables[2]);
            const nullishValues: (number | null | undefined)[] = [
                nullish(1), nullish(2), nullish(3)
            ];
            console.log(nullishValues[0]);
            console.log(nullishValues[1]);
            console.log(nullishValues[2]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tagged_nullish_arrays"),
        "1\nundefined\n1\n2\nnull\n2\n3\nnull\nundefined\n"
    );
}

#[test]
fn compiles_number_field_updates_with_single_object_evaluation() {
    let source = r#"
        function pointSource(point: { value: number }): { value: number } {
            console.log("source");
            return point;
        }
        async function asyncPoint(point: { value: number }): Promise<{ value: number }> {
            await sleep(1);
            console.log("async-source");
            return point;
        }
        async function main(): Promise<void> {
            let point = { value: 3 };
            console.log(pointSource(point).value++);
            console.log(point.value);
            console.log(++point.value);
            console.log((await asyncPoint(point)).value--);
            console.log(point.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "field_update_expression_values"),
        "source\n3\n4\n5\nasync-source\n5\n4\n"
    );
}

#[test]
fn compiles_dynamic_uniform_object_reads_as_optional_values() {
    let source = r#"
        function makePoint(): { a: number; b: number } {
            console.log("object");
            return { a: 1, b: 2 };
        }
        function selectedKey(): string {
            console.log("key");
            return "b";
        }
        function missingKey(): string { return "missing"; }
        async function asyncKey(): Promise<string> {
            await sleep(1);
            console.log("async-key");
            return "a";
        }
        async function main(): Promise<void> {
            console.log(makePoint()[selectedKey()]);
            const point = { a: 1, b: 2 };
            console.log(point[missingKey()]);
            console.log(point[await asyncKey()]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_computed_properties"),
        "object\nkey\n2\nundefined\nasync-key\n1\n"
    );
}

#[test]
fn deletes_runtime_keyed_dictionary_properties() {
    let source = r#"
        function objectSource(): Record<string, number> {
            console.log("object");
            return { transient: 1 };
        }
        async function asyncKey(): Promise<string> {
            await sleep(1);
            console.log("key");
            return "other";
        }
        async function main(): Promise<void> {
            const values: Record<string, number> = { answer: 42, other: 7 };
            const parsed: Json = JSON.parse("{\"answer\":42}");
            values[1] = 9;
            values[true] = 5;
            console.log(values[1]);
            console.log(values[true]);
            console.log(delete values[1]);
            console.log(delete values.answer);
            console.log(values["answer"]);
            console.log(delete values["missing"]);
            console.log(delete values[await asyncKey()]);
            console.log(values["other"]);
            console.log(delete objectSource().transient);
            console.log(delete parsed.answer);
            console.log(JSON.stringify(parsed));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "delete_dictionary_properties"),
        "9\n5\ntrue\ntrue\n0\ntrue\nkey\ntrue\n0\nobject\ntrue\ntrue\n{}\n"
    );
}

#[test]
fn reads_and_writes_json_with_runtime_string_keys() {
    let source = r#"
        function objectSource(): Json {
            console.log("object");
            return JSON.parse("{\"value\":1}");
        }
        function key(): string {
            console.log("key");
            return "value";
        }
        async function asyncKey(): Promise<string> {
            await sleep(1);
            console.log("async-key");
            return "other";
        }
        async function asyncIndex(): Promise<number> {
            await sleep(1);
            console.log("async-index");
            return 1;
        }
        async function main(): Promise<void> {
            const data: Json = JSON.parse("{\"value\":1,\"1\":\"one\",\"1.5\":\"fraction\"}");
            console.log(String(data[1]));
            console.log(String(data[1.5]));
            data[2] = JSON.parse("\"two\"");
            data[-1] = JSON.parse("\"negative\"");
            console.log(Number(data[key()]));
            data[key()] = JSON.parse("2");
            data.extra = JSON.parse("\"text\"");
            data[await asyncKey()] = JSON.parse("true");
            console.log(Number(data.value));
            console.log(String(data["extra"]));
            console.log(String(data.other));
            console.log(String(objectSource()[key()]));
            console.log(JSON.stringify(data));
            const array: Json = JSON.parse("[10,20]");
            console.log(Number(array[1.5]));
            array[await asyncIndex()] = JSON.parse("30");
            array[3] = JSON.parse("40");
            console.log(String(array[0] = JSON.parse("11")));
            console.log(Number(array[1]));
            console.log(JSON.stringify(array));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "runtime_json_keys"),
        "one\nfraction\nkey\n1\nkey\nasync-key\n2\ntext\ntrue\nobject\nkey\n1\n{\"1\":\"one\",\"2\":\"two\",\"value\":2,\"1.5\":\"fraction\",\"-1\":\"negative\",\"extra\":\"text\",\"other\":true}\n0\nasync-index\n11\n30\n[11,30,null,40]\n"
    );
}

#[test]
fn stringifies_json_with_number_and_string_spacing() {
    let source = r#"
        function value(): Json {
            console.log("value");
            return JSON.parse("{\"b\":2,\"a\":{\"x\":1}}");
        }
        async function spacing(): Promise<number> {
            await sleep(1);
            console.log("space");
            return 2.9;
        }
        async function keys(): Promise<string[]> {
            await sleep(1);
            console.log("replacer");
            return ["a", "x", "a"];
        }
        async function main(): Promise<void> {
            console.log(JSON.stringify(value(), null, await spacing()));
            console.log(JSON.stringify(JSON.parse("[1]"), undefined, "--"));
            console.log(JSON.stringify(JSON.parse("{\"x\":1}"), null, "abcdefghijkl"));
            console.log(JSON.stringify(JSON.parse("[1]"), null, 20));
            console.log(JSON.stringify(JSON.parse("{\"x\":1}"), null, -1));
            console.log(JSON.stringify(
                JSON.parse("{\"a\":{\"x\":1,\"y\":2},\"b\":3}"),
                ["b", "a", "x", "b"]
            ));
            console.log(JSON.stringify(value(), await keys(), "--"));
            console.log(JSON.stringify(JSON.parse("[{\"a\":1,\"b\":2}]"), ["a"]));
            const record: Record<string, number> = { answer: 42 };
            console.log(JSON.stringify(record));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "json_stringify_spacing"),
        "value\nspace\n{\n  \"b\": 2,\n  \"a\": {\n    \"x\": 1\n  }\n}\n[\n--1\n]\n{\nabcdefghij\"x\": 1\n}\n[\n          1\n]\n{\"x\":1}\n{\"b\":3,\"a\":{\"x\":1}}\nvalue\nreplacer\n{\n--\"a\": {\n----\"x\": 1\n--}\n}\n[{\"a\":1}]\n{\"answer\":42}\n"
    );
}

#[test]
fn stringifies_native_typed_values() {
    let source = r#"
        interface Point { x: number; y: number; }
        interface Shape { name: string; points: Point[]; }
        async function main(): Promise<void> {
            const shape: Shape = {
                name: "triangle",
                points: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 0, y: 1 }],
            };
            console.log(JSON.stringify(shape));
            const tuple: [number, string] = [42, "hi"];
            console.log(JSON.stringify(tuple));
            console.log(JSON.stringify([1, 2, 3]));
            console.log(JSON.stringify(42));
            console.log(JSON.stringify("hi"));
            console.log(JSON.stringify(true));
            console.log(JSON.stringify(shape, null, 2));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "json_stringify_native_values"),
        concat!(
            "{\"name\":\"triangle\",\"points\":[{\"x\":0,\"y\":0},{\"x\":1,\"y\":0},{\"x\":0,\"y\":1}]}\n",
            "[42,\"hi\"]\n",
            "[1,2,3]\n",
            "42\n",
            "\"hi\"\n",
            "true\n",
            "{\n  \"name\": \"triangle\",\n  \"points\": [\n    {\n      \"x\": 0,\n      \"y\": 0\n    },\n    {\n      \"x\": 1,\n      \"y\": 0\n    },\n    {\n      \"x\": 0,\n      \"y\": 1\n    }\n  ]\n}\n",
        )
    );
}

#[test]
fn rejects_json_stringify_function_replacers() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const value: Json = JSON.parse("{}");
            console.log(JSON.stringify(
                value,
                (key: string, current: Json): Json => current
            ));
        }"#,
    )
    .unwrap();
    let error = thaw_hir::lower_module(&module).unwrap_err();
    assert!(error.contains("function replacers"), "{error}");
}

#[test]
fn compiles_dynamic_heterogeneous_object_reads_as_unions() {
    let source = r#"
        function read(key: string): number | string | null | undefined {
            const mixed = {
                value: 41, label: "thaw", empty: null, absent: undefined
            };
            return mixed[key];
        }
        function readTagged(key: string): number | string | null | undefined {
            const mixed: {
                optionalValue: number | undefined;
                optionalAbsent: number | undefined;
                nullableValue: number | null;
                nullableNull: number | null;
                nullishValue: number | null | undefined;
                nullishNull: number | null | undefined;
                nullishUndefined: number | null | undefined;
                label: string;
            } = {
                optionalValue: 6,
                optionalAbsent: undefined,
                nullableValue: 7,
                nullableNull: null,
                nullishValue: 8,
                nullishNull: null,
                nullishUndefined: undefined,
                label: "tagged"
            };
            return mixed[key];
        }
        function print(key: string): void {
            const value = read(key);
            if (typeof value === "number") {
                console.log(value + 1);
            } else if (typeof value === "string") {
                console.log(value + "!");
            } else if (typeof value === "object") {
                console.log(value === null);
            } else {
                console.log(value);
            }
        }
        function main(): void {
            print("value");
            print("label");
            print("empty");
            print("absent");
            print("missing");
            console.log(readTagged("optionalValue"));
            console.log(readTagged("optionalAbsent"));
            console.log(readTagged("nullableValue"));
            console.log(readTagged("nullableNull"));
            console.log(readTagged("nullishValue"));
            console.log(readTagged("nullishNull"));
            console.log(readTagged("nullishUndefined"));
            console.log(readTagged("label"));
            console.log(readTagged("missing"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_heterogeneous_properties"),
        "42\nthaw!\ntrue\nundefined\nundefined\n6\nundefined\n7\nnull\n8\nnull\nundefined\ntagged\nundefined\n"
    );
}

#[test]
fn compiles_dynamic_uniform_tagged_object_reads_without_nested_tags() {
    let source = r#"
        function key(value: string): string { return value; }
        async function delayedKey(): Promise<string> {
            await sleep(1);
            return "nullValue";
        }
        async function main(): Promise<void> {
            const optional: {
                value: number | undefined;
                absent: number | undefined;
            } = { value: 1, absent: undefined };
            console.log(optional[key("value")]);
            console.log(optional[key("absent")]);
            console.log(optional[key("missing")]);

            const nullable: {
                value: number | null;
                nullValue: number | null;
            } = { value: 2, nullValue: null };
            console.log(nullable[key("value")]);
            console.log(nullable[await delayedKey()]);
            console.log(nullable[key("missing")]);

            const nullish: {
                value: number | null | undefined;
                nullValue: number | null | undefined;
                absent: number | null | undefined;
            } = { value: 3, nullValue: null, absent: undefined };
            console.log(nullish[key("value")]);
            console.log(nullish[key("nullValue")]);
            console.log(nullish[key("absent")]);
            console.log(nullish[key("missing")]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_tagged_computed_properties"),
        "1\nundefined\nundefined\n2\nnull\nundefined\n3\nnull\nundefined\nundefined\n"
    );
}

#[test]
fn compiles_numeric_and_string_enums_as_typed_constants() {
    let source = r#"
        function lookup(): number {
            console.log("lookup");
            return 4;
        }
        async function delayedIndex(): Promise<number> {
            await sleep(1);
            return 6;
        }
        function adjust(direction: Direction): number {
            return direction + Direction.Next;
        }
        function label(value: Label): string { return value; }
        async function main(): Promise<void> {
            console.log(Direction.None);
            console.log(Direction.Up);
            console.log(Direction.Next);
            console.log(Direction["Mask"]);
            console.log(Direction[lookup()]);
            console.log(Direction[99]);
            console.log(Direction[await delayedIndex()]);
            console.log(adjust(Direction.None));
            console.log(label(Label.Ready));
            console.log(Label.Alias === Label.Ready);
        }
        enum Direction { None, Up = 4, Next = Up + 2, Mask = 1 << 3, Alias = 4 }
        enum Label { Ready = "ready", Alias = Ready }
    "#;
    assert_eq!(
        compile_and_run(source, "typed_enums"),
        "0\n4\n6\n8\nlookup\nAlias\nundefined\nNext\n6\nready\ntrue\n"
    );
}

#[test]
fn compiles_merged_enum_declarations() {
    let source = r#"
        enum Status {}
        enum Status { First }
        enum Status { Second }
        enum Status { Third = 2 }
        enum Label { First = "first" }
        enum Label { Second = "second" }
        function main(): void {
            console.log(Status.First);
            console.log(Status.Second);
            console.log(Status.Third);
            console.log(Status[0]);
            console.log(Status[2]);
            console.log(Label.First);
            console.log(Label.Second);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "merged_enums"),
        "0\n0\n2\nSecond\nThird\nfirst\nsecond\n"
    );
}

#[test]
fn compiles_same_layout_literal_unions_across_native_layouts() {
    let source = r#"
        function command(value: "start" | "stop"): string { return value; }
        function numberValue(value: 1 | 2 | number): number { return value; }
        function flag(value: true | false): boolean { return value; }
        function fixed(value: string & "fixed"): string { return value; }
        function state(value: { kind: "ready" | "waiting" }): string {
            return value.kind;
        }
        async function delayed(value: "later" | "now"): Promise<string> {
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            console.log(command("start"));
            console.log(command("stop"));
            console.log(numberValue(2));
            console.log(flag(true));
            console.log(fixed("fixed"));
            console.log(state({ kind: "waiting" }));
            const values: ("a" | "b")[] = ["a", "b"];
            console.log(values.join(","));
            console.log(await delayed("later"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "same_layout_literal_unions"),
        "start\nstop\n2\ntrue\nfixed\nwaiting\na,b\nlater\n"
    );
}

#[test]
fn compiles_compatible_object_intersections() {
    let source = r#"
        function inspect(
            value: { name: string } & { count: number } & { enabled: boolean }
        ): string {
            return value.name + ":" + String(value.count) + ":" + String(value.enabled);
        }
        async function delayed(
            value: { name: string } & { count: number }
        ): Promise<string> {
            await sleep(1);
            return value.name + String(value.count);
        }
        async function main(): Promise<void> {
            console.log(inspect({ enabled: true, count: 3, name: "item" }));
            console.log(await delayed({ count: 4, name: "later" }));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "compatible_object_intersections"),
        "item:3:true\nlater4\n"
    );
}

#[test]
fn compiles_heterogeneous_union_arrays_with_wide_elements() {
    let source = r#"
        function kind(value: string | number): string { return typeof value; }
        function main(): void {
            const values: (string | number)[] = ["a", 2, "b", 4];
            console.log(values.length);
            console.log(kind(values[0]));
            console.log(kind(values[1]));
            console.log(kind(values[2]));
            const combined: (string | number)[] = [...values, "end", 5];
            console.log(combined.length);
            console.log(kind(combined[4]));
            console.log(kind(combined[5]));
            console.log(kind(values[3]));
            values[1] = "two";
            values[2] = 3;
            console.log(kind(values[1]));
            console.log(kind(values[2]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "heterogeneous_union_arrays"),
        "4\nstring\nnumber\nstring\n6\nstring\nnumber\nnumber\nstring\nnumber\n"
    );
}

#[test]
fn compiles_null_members_in_general_unions() {
    let source = r#"
        function choose(kind: number): number | string | null {
            if (kind === 0) return 7;
            if (kind === 1) return "value";
            return null;
        }
        function describe(value: number | string | null): string {
            if (typeof value === "number") return String(value + 1);
            if (typeof value === "string") return value + "!";
            console.log(value === null);
            return "null";
        }
        function reorder(value: string | null | number): number | string | null {
            return value;
        }
        function afterGuards(
            value: number | string | null | undefined
        ): string {
            if (value === null) return "null";
            if (undefined === value) return "undefined";
            if (typeof value === "string") return value + "!";
            return String(value + 1);
        }
        function literalNarrowing(value: number | string | boolean): string {
            if (value === 0) return String(value + 1);
            if ("exact" === value) return value + "!";
            if (value === true) return value ? "true" : "false";
            if (typeof value === "number") return String(value * 2);
            if (typeof value === "string") return value + "?";
            return value ? "yes" : "no";
        }
        function notEqualNarrowing(value: number | string): string {
            if (value !== 1) {
                if (typeof value === "number") return String(value + 1);
                return value + "!";
            }
            return String(value + 10);
        }
        async function delayed(kind: number): Promise<number | string | null> {
            await sleep(1);
            return choose(kind);
        }
        async function delayedReorder(
            value: null | string | number
        ): Promise<string | number | null> {
            await sleep(1);
            return value;
        }
        function equalAcrossOrders(
            left: number | string | null,
            right: null | string | number
        ): boolean {
            return left === right;
        }
        async function equalAcrossOrdersAsync(
            left: string | null | number,
            right: number | string | null
        ): Promise<boolean> {
            await sleep(1);
            return left === right;
        }
        async function main(): Promise<void> {
            console.log(describe(choose(0)));
            console.log(describe(choose(1)));
            console.log(describe(choose(2)));
            console.log(choose(2));
            console.log(describe(await delayed(2)));
            console.log(describe(reorder(9)));
            console.log(describe(reorder("ordered")));
            console.log(describe(reorder(null)));
            console.log(describe(await delayedReorder(10)));
            console.log(describe(await delayedReorder("async")));
            console.log(describe(await delayedReorder(null)));
            console.log(afterGuards(12));
            console.log(afterGuards("guard"));
            console.log(afterGuards(null));
            console.log(afterGuards(undefined));
            console.log(equalAcrossOrders(4, 4));
            console.log(equalAcrossOrders("same", "same"));
            console.log(equalAcrossOrders(null, null));
            console.log(equalAcrossOrders(4, "4"));
            console.log(equalAcrossOrders(NaN, NaN));
            console.log(await equalAcrossOrdersAsync("async", "async"));
            console.log(await equalAcrossOrdersAsync(null, null));
            console.log(literalNarrowing(0));
            console.log(literalNarrowing(3));
            console.log(literalNarrowing("exact"));
            console.log(literalNarrowing("other"));
            console.log(literalNarrowing(true));
            console.log(literalNarrowing(false));
            console.log(notEqualNarrowing(1));
            console.log(notEqualNarrowing(2));
            console.log(notEqualNarrowing("text"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "general_union_null_members"),
        "8\nvalue!\ntrue\nnull\nnull\ntrue\nnull\n10\nordered!\ntrue\nnull\n11\nasync!\ntrue\nnull\n13\nguard!\nnull\nundefined\ntrue\ntrue\ntrue\nfalse\nfalse\ntrue\ntrue\n1\n6\nexact!\nother?\ntrue\nno\n11\n3\ntext!\n"
    );
}

#[test]
fn compiles_object_union_properties_and_discriminant_type_narrowing() {
    let source = r#"
        type Result =
            { kind: number; value: number; shared: string } |
            { kind: string; value: string; shared: string };
        function describe(result: Result): string {
            console.log(result.shared);
            if (typeof result.kind === "number") {
                return String(result.value + 1);
            }
            return result.value + "!";
        }
        async function delayed(flag: boolean): Promise<Result> {
            await sleep(1);
            if (flag) return { kind: 1, value: 41, shared: "number" };
            return { kind: "text", value: "thaw", shared: "string" };
        }
        async function main(): Promise<void> {
            console.log(describe({ kind: 0, value: 2, shared: "sync-number" }));
            console.log(describe({ kind: "text", value: "sync", shared: "sync-string" }));
            console.log(describe(await delayed(true)));
            console.log(describe(await delayed(false)));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_union_discriminants"),
        "sync-number\n3\nsync-string\nsync!\nnumber\n42\nstring\nthaw!\n"
    );
}

#[test]
fn flattens_tagged_properties_across_object_union_members() {
    let source = r#"
        type Mixed =
            { kind: number; value: number | undefined } |
            { kind: string; value: string | null } |
            { kind: boolean; value: boolean | null | undefined } |
            { kind: Json; value: number | string };
        function show(value: Mixed): void { console.log(value.value); }
        async function delayed(kind: number): Promise<Mixed> {
            await sleep(1);
            if (kind === 0) return { kind: 0, value: undefined };
            if (kind === 1) return { kind: "text", value: null };
            if (kind === 2) return { kind: true, value: true };
            return { kind: JSON.parse("{}"), value: "nested" };
        }
        async function main(): Promise<void> {
            show({ kind: 0, value: 41 });
            show({ kind: 0, value: undefined });
            show({ kind: "text", value: "thaw" });
            show({ kind: "text", value: null });
            show({ kind: false, value: false });
            show({ kind: false, value: undefined });
            show({ kind: JSON.parse("{}"), value: 7 });
            show({ kind: JSON.parse("{}"), value: "nested" });
            show(await delayed(0));
            show(await delayed(1));
            show(await delayed(2));
            show(await delayed(3));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "flattened_object_union_properties"),
        "41\nundefined\nthaw\nnull\nfalse\nundefined\n7\nnested\nundefined\nnull\ntrue\nnested\n"
    );
}

#[test]
fn narrows_object_unions_by_literal_discriminants() {
    let source = r#"
        type Result =
            { kind: "success"; value: number } |
            { kind: "failure"; value: string } |
            { kind: true; value: boolean };
        type Numeric =
            { code: 200; value: number } |
            { code: 500; value: string };
        type Factory = (ok: boolean) => Result;
        type AsyncFactory = (ok: boolean) => Promise<Result>;
        function describe(result: Result): string {
            if (result.kind === "success") return String(result.value + 1);
            if ("failure" === result.kind) return result.value + "!";
            return result.value ? "true" : "false";
        }
        function numeric(result: Numeric): string {
            if (result.code !== 200) return result.value + "!";
            return String(result.value + 2);
        }
        function local(ok: boolean): string {
            let result: Result = { kind: "success", value: 9 };
            if (!ok) result = { kind: "failure", value: "local" };
            if (!(result.kind !== "success")) return String(result.value + 1);
            if (result.kind === "failure") return result.value + "!";
            return result.value ? "true" : "false";
        }
        function make(ok: boolean): Result {
            if (ok) return { kind: "success", value: 20 };
            return { kind: "failure", value: "returned" };
        }
        function inferred(ok: boolean): string {
            const returned = make(ok);
            const alias = returned;
            if (alias.kind === "success") return String(alias.value + 2);
            if (alias.kind === "failure") return alias.value + "!";
            return "boolean";
        }
        function asserted(): string {
            const result = ({ kind: "success", value: 30 } as Result);
            if (result.kind === "success") return String(result.value + 3);
            return "wrong";
        }
        function compound(result: Result): string {
            if (result.kind === "success" && result.value > 0) {
                return String(result.value + 1);
            }
            if (result.kind !== "failure" || result.value === "expected") {
                return "accepted";
            }
            return result.value + "!";
        }
        function consume(factory: Factory, ok: boolean): string {
            const result = factory(ok);
            if (result.kind === "success") return String(result.value + 4);
            if (result.kind === "failure") return result.value + " callback";
            return "boolean";
        }
        function throughFunctionValue(ok: boolean): string {
            const factory: Factory = (value: boolean): Result => {
                if (value) return { kind: "success", value: 40 };
                return { kind: "failure", value: "factory" };
            };
            const alias = factory;
            return consume(alias, ok);
        }
        async function asyncFactory(value: boolean): Promise<Result> {
            await sleep(1);
            if (value) return { kind: "success", value: 50 };
            return { kind: "failure", value: "async-factory" };
        }
        async function throughAsyncFunctionValue(ok: boolean): Promise<string> {
            const factory: AsyncFactory = asyncFactory;
            const result = await factory(ok);
            if (result.kind === "success") return String(result.value + 5);
            if (result.kind === "failure") return result.value + " callback";
            return "boolean";
        }
        async function delayed(ok: boolean): Promise<Result> {
            await sleep(1);
            if (ok) return { kind: "success", value: 41 };
            return { kind: "failure", value: "async" };
        }
        async function main(): Promise<void> {
            console.log(describe({ kind: "success", value: 2 }));
            console.log(describe({ kind: "failure", value: "sync" }));
            console.log(describe({ kind: true, value: false }));
            console.log(numeric({ code: 200, value: 5 }));
            console.log(numeric({ code: 500, value: "error" }));
            console.log(local(true));
            console.log(local(false));
            console.log(inferred(true));
            console.log(inferred(false));
            console.log(asserted());
            console.log(compound({ kind: "success", value: 4 }));
            console.log(compound({ kind: "failure", value: "expected" }));
            console.log(compound({ kind: "failure", value: "rejected" }));
            console.log(throughFunctionValue(true));
            console.log(throughFunctionValue(false));
            console.log(await throughAsyncFunctionValue(true));
            console.log(await throughAsyncFunctionValue(false));
            console.log(describe(await delayed(true)));
            console.log(describe(await delayed(false)));
            const awaited = await delayed(false);
            if (awaited.kind === "failure") console.log(awaited.value + " inferred");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "literal_object_union_discriminants"),
        "3\nsync!\nfalse\n7\nerror!\n10\nlocal!\n22\nreturned!\n33\n5\naccepted\nrejected!\n44\nfactory callback\n55\nasync-factory callback\n42\nasync!\nasync inferred\n"
    );
}

#[test]
fn destructures_object_unions_with_nested_fields_defaults_and_rest() {
    let source = r#"
        type Event =
            { kind: "number"; value: number; nested: { detail: number }; optional: number | undefined } |
            { kind: "text"; value: string; nested: { detail: string }; optional: string | undefined };
        function source(event: Event): Event {
            console.log("source");
            return event;
        }
        async function delayed(): Promise<Event> {
            await sleep(1);
            return { kind: "text", value: "async", nested: { detail: "nested" }, optional: undefined };
        }
        function show(event: Event): void {
            const { kind: category, optional = "default", ...rest } = source(event);
            console.log(category);
            console.log(optional);
            console.log(rest.value);
            console.log(rest.nested.detail);
        }
        async function main(): Promise<void> {
            show({ kind: "number", value: 7, nested: { detail: 8 }, optional: 9 });
            show({ kind: "text", value: "value", nested: { detail: "detail" }, optional: undefined });
            const { kind, value, nested: { detail }, ...rest } = await delayed();
            console.log(kind);
            console.log(value);
            console.log(detail);
            console.log(rest.optional);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_union_destructuring"),
        "source\nnumber\n9\n7\n8\nsource\ntext\ndefault\nvalue\ndetail\ntext\nasync\nnested\nundefined\n"
    );
}

#[test]
fn correlates_destructured_discriminants_with_sibling_payloads() {
    let source = r#"
        type Result =
            { kind: "success"; value: number; detail: number } |
            { kind: "failure"; value: string; detail: string } |
            { kind: "pending"; value: boolean; detail: boolean };
        type Numeric =
            { code: 200; value: number } |
            { code: 500; value: string };
        type Flagged =
            { active: true; value: number } |
            { active: false; value: string };
        type Nested =
            { kind: "success"; nested: { value: number }; extra: number } |
            { kind: "failure"; nested: { value: string }; extra: string };
        type Defaulted =
            { kind: "number"; value: number } |
            { kind: "text"; value: string | undefined };
        type TupleNested =
            { kind: "number"; pair: [number, number, number] } |
            { kind: "text"; pair: [string, string, string] };
        type TupleDefault =
            { kind: "number"; pair: [number] } |
            { kind: "text"; pair: [string | undefined] };
        function describe(result: Result): string {
            const { kind: tag, value, detail } = result;
            if (tag === "success" && value > 0) {
                return String(value + detail);
            }
            if ("failure" === tag) return value + detail;
            if (tag === "pending") return value && detail ? "pending" : "waiting";
            return "zero";
        }
        function guarded(result: Result): string {
            const { kind, value, detail } = result;
            if (kind !== "success") {
                if (!(kind !== "failure")) return value + detail;
                return value || detail ? "pending" : "waiting";
            }
            return String(value + detail);
        }
        function numeric(result: Numeric): string {
            const { code, value } = result;
            if (code === 200) return String(value + 1);
            return value + "!";
        }
        function flagged(result: Flagged): string {
            const { active, value } = result;
            if (active === true) return String(value + 2);
            return value + "!";
        }
        function parameter({ kind, value }: Result): string {
            if (kind === "success") return String(value + 3);
            if (kind === "failure") return value + " parameter";
            return value ? "pending" : "waiting";
        }
        function nested(result: Nested): string {
            const { kind, nested: { value }, ...rest } = result;
            if (kind === "success") return String(value + rest.extra);
            return value + rest.extra;
        }
        function defaulted(result: Defaulted): string {
            const { kind, value = "missing" } = result;
            if (kind === "number") return String(value + 1);
            return value + "!";
        }
        function tupleNested(result: TupleNested): string {
            const { kind, pair: [first, ...tail] } = result;
            if (kind === "number") return String(first + tail[0]);
            return first + tail[0];
        }
        function tupleDefault(result: TupleDefault): string {
            const { kind, pair: [value = "missing"] } = result;
            if (kind === "number") return String(value + 2);
            return value + "!";
        }
        function aliased(result: Result): string {
            const { kind, value } = result;
            const tagAlias = (kind);
            const payloadAlias = (value as string | number | boolean);
            if (tagAlias === "success") return String(payloadAlias + 4);
            if (tagAlias === "failure") return payloadAlias + " alias";
            return payloadAlias ? "pending" : "waiting";
        }
        function switched(result: Result): string {
            const { kind, value, detail } = result;
            switch (kind) {
                case "success": return String(value + detail);
                case "failure": return value + detail;
                case "pending": return value && detail ? "pending" : "waiting";
                default: return "unknown";
            }
            return "unreachable";
        }
        function switchedObject(result: Result): string {
            switch (result.kind) {
                case "success": return String(result.value + 5);
                case "failure": return result.value + " switch";
                case "pending": return result.value ? "pending" : "waiting";
                default: return "unknown";
            }
            return "unreachable";
        }
        function switchDefault(result: Result): string {
            const { kind, value } = result;
            switch (kind) {
                case "success": return String(value + 6);
                case "failure": return value + " default";
                default: return value ? "pending" : "waiting";
            }
            return "unreachable";
        }
        async function delayed(ok: boolean): Promise<Result> {
            await sleep(1);
            if (ok) return { kind: "success", value: 20, detail: 2 };
            return { kind: "failure", value: "async", detail: "!" };
        }
        async function main(): Promise<void> {
            console.log(describe({ kind: "success", value: 4, detail: 3 }));
            console.log(describe({ kind: "failure", value: "bad", detail: "!" }));
            console.log(describe({ kind: "pending", value: true, detail: false }));
            console.log(guarded({ kind: "success", value: 8, detail: 1 }));
            console.log(guarded({ kind: "failure", value: "no", detail: "?" }));
            console.log(guarded({ kind: "pending", value: false, detail: false }));
            console.log(numeric({ code: 200, value: 5 }));
            console.log(numeric({ code: 500, value: "error" }));
            console.log(flagged({ active: true, value: 8 }));
            console.log(flagged({ active: false, value: "off" }));
            console.log(parameter({ kind: "success", value: 7, detail: 0 }));
            console.log(parameter({ kind: "failure", value: "bad", detail: "" }));
            console.log(nested({ kind: "success", nested: { value: 6 }, extra: 4 }));
            console.log(nested({ kind: "failure", nested: { value: "nested" }, extra: "!" }));
            console.log(defaulted({ kind: "number", value: 4 }));
            console.log(defaulted({ kind: "text", value: undefined }));
            console.log(tupleNested({ kind: "number", pair: [3, 4, 5] }));
            console.log(tupleNested({ kind: "text", pair: ["tuple", "!", "?"] }));
            console.log(tupleDefault({ kind: "number", pair: [8] }));
            console.log(tupleDefault({ kind: "text", pair: [undefined] }));
            console.log(aliased({ kind: "success", value: 6, detail: 0 }));
            console.log(aliased({ kind: "failure", value: "bad", detail: "" }));
            console.log(switched({ kind: "success", value: 5, detail: 2 }));
            console.log(switched({ kind: "failure", value: "switch", detail: "!" }));
            console.log(switched({ kind: "pending", value: true, detail: true }));
            console.log(switchedObject({ kind: "success", value: 5, detail: 0 }));
            console.log(switchedObject({ kind: "failure", value: "bad", detail: "" }));
            console.log(switchDefault({ kind: "pending", value: true, detail: false }));
            const { kind, value, detail } = await delayed(true);
            if (kind === "success") console.log(value + detail);
            const { kind: asyncKind, value: asyncValue, detail: asyncDetail } = await delayed(false);
            if (asyncKind !== "success") {
                if (asyncKind === "failure") console.log(asyncValue + asyncDetail);
            }
            let assignedNestedKind: string = "failure";
            let assignedNestedValue: number | string = "initial";
            let assignedNestedExtra: number | string = "initial";
            const nestedSource: Nested = {
                kind: "success", nested: { value: 7 }, extra: 8
            };
            ({
                kind: assignedNestedKind,
                nested: { value: assignedNestedValue },
                extra: assignedNestedExtra
            } = nestedSource);
            if (assignedNestedKind === "success") {
                console.log(assignedNestedValue + assignedNestedExtra);
            }
            let assignedDefaultKind: string = "number";
            let assignedDefaultValue: number | string = 0;
            const defaultSource: Defaulted = { kind: "text", value: undefined };
            ({
                kind: assignedDefaultKind,
                value: assignedDefaultValue = "assigned missing"
            } = defaultSource);
            if (assignedDefaultKind === "text") console.log(assignedDefaultValue + "!");
            let assignedRestKind: string = "success";
            let assignedRest:
                { value: number; detail: number } |
                { value: string; detail: string } |
                { value: boolean; detail: boolean } = { value: 0, detail: 0 };
            const restSource: Result = {
                kind: "failure", value: "rest", detail: "!"
            };
            ({ kind: assignedRestKind, ...assignedRest } = restSource);
            if (assignedRestKind === "failure") {
                console.log(assignedRest.value + assignedRest.detail);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "correlated_object_union_destructuring"),
        "7\nbad!\nwaiting\n9\nno?\nwaiting\n6\nerror!\n10\noff!\n10\nbad parameter\n10\nnested!\n5\nmissing!\n7\ntuple!\n10\nmissing!\n10\nbad alias\n7\nswitch!\npending\n10\nbad switch\npending\n22\nasync!\n15\nassigned missing!\nrest!\n"
    );
}

#[test]
fn compiles_destructuring_defaults_for_optionals() {
    let source = r#"
        interface Options {
            count: number | undefined;
            label: string | undefined;
        }
        function fallback(): number {
            console.log("fallback");
            return 7;
        }
        async function delayedFallback(): Promise<number> {
            console.log("delayed fallback");
            await sleep(1);
            return 9;
        }
        async function main(): Promise<void> {
            const missing: Options = { count: undefined, label: undefined };
            const present: Options = { count: 3, label: "ok" };
            const { count = fallback(), label = "default" } = missing;
            console.log(count);
            console.log(label);
            const { count: kept = fallback(), label: keptLabel = "wrong" } = present;
            console.log(kept);
            console.log(keptLabel);

            const tuple: [number | undefined, number | undefined] = [undefined, 4];
            const [first = fallback(), second = fallback()] = tuple;
            console.log(first);
            console.log(second);

            let assigned = 0;
            let other = 0;
            [assigned = fallback(), other = fallback()] = tuple;
            console.log(assigned);
            console.log(other);
            ({ count: assigned = fallback() } = present);
            console.log(assigned);

            const asyncMissing: [number | undefined] = [undefined];
            const asyncPresent: [number | undefined] = [6];
            const [delayed = await delayedFallback()] = asyncMissing;
            const [notDelayed = await delayedFallback()] = asyncPresent;
            console.log(delayed);
            console.log(notDelayed);

            const nullable: [number | null] = [null];
            const [keptNull = fallback()] = nullable;
            console.log(keptNull);
            const nullish: [number | null | undefined, number | null | undefined] =
                [null, undefined];
            const [alsoNull = fallback(), defaultedUndefined = fallback()] = nullish;
            console.log(alsoNull);
            console.log(defaultedUndefined);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "destructuring_defaults"),
        "fallback\n7\ndefault\n3\nok\nfallback\n7\n4\nfallback\n7\n4\n3\ndelayed fallback\n9\n6\nnull\nfallback\nnull\n7\n"
    );
}

#[test]
fn compiles_destructuring_assignments_and_returns_rhs() {
    let source = r#"
        async function source(): Promise<{
            x: number; nested: { flag: boolean }; label: string; extra: number
        }> {
            await sleep(1);
            console.log("source");
            return { x: 1, nested: { flag: true }, label: "ok", extra: 4 };
        }
        function tupleSource(): [number, number, number] {
            console.log("tuple");
            return [5, 6, 7];
        }
        async function main(): Promise<void> {
            let x = 0;
            let flag = false;
            let rest = { label: "", extra: 0 };
            const returned = ({ x, nested: { flag }, ...rest } = await source());
            console.log(x);
            console.log(flag);
            console.log(rest.label);
            console.log(rest.extra);
            console.log(returned.x);
            let first = 0;
            let tail = [0, 0];
            [first, ...tail] = tupleSource();
            console.log(first);
            console.log(tail[0]);
            console.log(tail[1]);
            const returnedTuple = ([first, ...tail] = [8, 9, 10]);
            console.log(returnedTuple[0]);
            console.log(first);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "destructuring_assignments"),
        "source\n1\ntrue\nok\n4\n1\ntuple\n5\n6\n7\n8\n8\n"
    );
}

#[test]
fn compiles_optional_chains_for_non_null_native_types() {
    let source = r#"
        function invoke(callback: (value: number) => number): number {
            return callback?.(2);
        }
        async function boxValue(): Promise<{ value: number }> {
            await sleep(1); return { value: 4 };
        }
        function callbackValue(): number {
            const add = (value: number): number => value + 1;
            return invoke(add);
        }
        async function main(): Promise<void> {
            const box = { value: 1 };
            console.log(box?.value);
            console.log(box?.["value"]);
            console.log(callbackValue());
            console.log((await boxValue())?.value);
            const data: Json = JSON.parse("{\"name\":\"thaw\"}");
            console.log(String(data?.name));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "non_null_optional_chains"),
        "1\n1\n3\n4\nthaw\n"
    );
}

#[test]
fn compiles_fixed_object_in_checks_in_operand_order() {
    let source = r#"
        async function object(label: string): Promise<{ value: number; flag: boolean }> {
            await sleep(1); console.log(label); return { value: 1, flag: true };
        }
        function key(): string { console.log("key"); return "value"; }
        async function json(): Promise<Json> {
            await sleep(1); console.log("json"); return JSON.parse("{\"value\":1}");
        }
        async function main(): Promise<void> {
            console.log("value" in (await object("first")));
            console.log("missing" in (await object("second")));
            console.log(key() in (await object("third")));
            console.log(1 in { "1": true });
            console.log(key() in (await json()));
            const record: Record<string, number> = { value: 1 };
            console.log("value" in record);
            console.log("missing" in record);
            console.log("missing" in JSON.parse("[10,20]"));
            console.log("length" in JSON.parse("[10,20]"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "fixed_object_in"),
        "first\ntrue\nsecond\nfalse\nkey\nthird\ntrue\ntrue\nkey\njson\ntrue\ntrue\nfalse\nfalse\ntrue\n"
    );
}

#[test]
fn compiles_object_and_typed_array_string_conversion() {
    let source = r#"
        function objectValue(): { value: number } {
            console.log("object-evaluated");
            return { value: 1 };
        }
        function main(): void {
            const numbers: number[] = [1, -0, 2.5, 1000000000000000000000];
            const words: string[] = ["a", "", "c"];
            const flags: boolean[] = [true, false, true];
            const objects: { value: number }[] = [{ value: 1 }, { value: 2 }];
            console.log(String(numbers));
            console.log(`words=${words}`);
            console.log("flags=" + flags);
            console.log(String(objects));
            console.log(String(objectValue()));
            console.log(`object=${{ value: 3 }}`);
            const empty: number[] = [];
            console.log(`empty=${empty}`);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "aggregate_string_conversion"),
        "1,0,2.5,1e+21\nwords=a,,c\nflags=true,false,true\n[object Object],[object Object]\nobject-evaluated\n[object Object]\nobject=[object Object]\nempty=\n"
    );
}

#[test]
fn compiles_heterogeneous_tuple_string_conversion_once() {
    let source = r#"
        function tupleValue(): [number, string, boolean, { value: number }, number[], [string, boolean]] {
            console.log("tuple-evaluated");
            return [1.5, "word", true, { value: 2 }, [3, 4], ["inner", false]];
        }
        function main(): void {
            console.log(String(tupleValue()));
            const tuple: [number, string, boolean] = [7, "x", false];
            console.log(`tuple=${tuple}`);
            console.log("prefix:" + tuple);
            const empty: [] = [];
            console.log(`empty-tuple=${empty}`);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tuple_string_conversion"),
        "tuple-evaluated\n1.5,word,true,[object Object],3,4,inner,false\ntuple=7,x,false\nprefix:7,x,false\nempty-tuple=\n"
    );
}

#[test]
fn compiles_in_place_array_copy_within() {
    let source = r#"
        function values(): number[] {
            console.log("receiver");
            return [1, 2, 3, 4, 5];
        }
        function index(label: string, value: string): string {
            console.log(label);
            return value;
        }
        async function delayedEnd(): Promise<number> {
            await sleep(1);
            console.log("awaited-end");
            return 4;
        }
        async function main(): Promise<void> {
            const tail: number[] = [1, 2, 3, 4, 5];
            console.log(tail.copyWithin(0, 3).join(","));
            const overlap: number[] = [1, 2, 3, 4, 5];
            overlap.copyWithin(1, 0, 4);
            console.log(overlap.join(","));
            const negative: number[] = [1, 2, 3, 4, 5];
            console.log(negative.copyWithin(-2, -4, -1).join(","));
            const words: string[] = ["a", "b", "c"];
            console.log(words.copyWithin(1, 0).join(""));
            console.log(values().copyWithin(index("target", "1"), index("start", "0"), await delayedEnd()).join("-"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_copy_within"),
        "4,5,3,4,5\n1,1,2,3,4\n1,2,3,2,3\naab\nreceiver\ntarget\nstart\nawaited-end\n1-1-2-3-4\n"
    );
}

#[test]
fn compiles_union_array_fill_without_losing_discriminants() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        function main(): void {
            const replacement: Result = { kind: "text", value: "filled" };
            const results: Result[] = [
                { kind: "number", value: 1 },
                { kind: "number", value: 2 },
                { kind: "number", value: 3 }
            ];
            const filled = results.fill(replacement, 1);
            for (const item of filled) {
                if (item.kind === "number") console.log(item.value + 10);
                else console.log(item.value + "!");
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "union_array_fill"),
        "11\nfilled!\nfilled!\n"
    );
}

#[test]
fn compiles_non_mutating_array_to_reversed() {
    let source = r#"
        async function delayed(): Promise<string[]> {
            await sleep(1);
            console.log("awaited-array");
            return ["a", "b", "c"];
        }
        async function main(): Promise<void> {
            const numbers: number[] = [1, 2, 3];
            const reversed: number[] = numbers.toReversed();
            console.log(reversed.join(","));
            console.log(numbers.join(","));
            const objects: { value: number }[] = [{ value: 1 }, { value: 2 }];
            const objectCopy: { value: number }[] = objects.toReversed();
            objectCopy[0].value = 9;
            console.log(objects[1].value);
            const empty: number[] = [];
            console.log(empty.toReversed().length);
            console.log((await delayed()).toReversed().join(""));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_to_reversed"),
        "3,2,1\n1,2,3\n9\n0\nawaited-array\ncba\n"
    );
}

#[test]
fn compiles_union_array_sorting_with_a_comparator() {
    let source = r#"
        type Result =
            { kind: "number"; value: number; rank: number } |
            { kind: "text"; value: string; rank: number };
        function print(values: Result[]): void {
            for (const item of values) {
                if (item.kind === "number") console.log(item.value + 10);
                else console.log(item.value + "!");
            }
        }
        function main(): void {
            const results: Result[] = [
                { kind: "text", value: "last", rank: 3 },
                { kind: "number", value: 1, rank: 1 },
                { kind: "text", value: "middle", rank: 2 }
            ];
            const sorted = results.toSorted((left, right) => left.rank - right.rank);
            print(sorted);
            print(results);
            results.sort((left, right) => left.rank - right.rank);
            print(results);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "union_array_sort"),
        "11\nmiddle!\nlast!\nlast!\n11\nmiddle!\n11\nmiddle!\nlast!\n"
    );
}

#[test]
fn compiles_optional_nullish_coalescing() {
    let source = r#"
        function fallback(): number {
            console.log("fallback");
            return 9;
        }
        async function delayedFallback(): Promise<string> {
            console.log("awaited fallback");
            await sleep(1);
            return "later";
        }
        async function main(): Promise<void> {
            console.log([1, 2].find(value => value === 1) ?? fallback());
            console.log([1, 2].find(value => value === 3) ?? fallback());
            const missing: string | undefined = ["a"].find(value => value === "b");
            const present: string | undefined = ["a"].find(value => value === "a");
            console.log(present ?? "wrong");
            console.log(missing ?? "default");
            console.log(([2, 3].at(0) ?? 0) + ([2, 3].at(9) ?? 4));
            console.log(present ?? await delayedFallback());
            console.log(missing ?? await delayedFallback());
            console.log(42 ?? fallback());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_nullish"),
        "1\nfallback\n9\na\ndefault\n6\na\nawaited fallback\nlater\n42\n"
    );
}

#[test]
fn compiles_optional_nullish_assignment() {
    let source = r#"
        interface Box { value: number | undefined; }
        function fallback(): number {
            console.log("fallback");
            return 9;
        }
        async function delayedFallback(): Promise<number> {
            console.log("awaited fallback");
            await sleep(1);
            return 12;
        }
        async function main(): Promise<void> {
            let value: number | undefined = undefined;
            console.log(value ??= fallback());
            console.log(value ??= fallback());
            console.log(value);
            value = undefined;
            console.log(value ??= await delayedFallback());
            console.log(value);

            const box: Box = { value: undefined };
            console.log(box.value ??= 5);
            console.log(box.value ??= fallback());
            console.log(box.value);

            let plain: number = 4;
            console.log(plain ??= fallback());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_nullish_assignment"),
        "fallback\n9\n9\n9\nawaited fallback\n12\n12\n5\n5\n5\n4\n"
    );
}

#[test]
fn compiles_optional_undefined_branch_narrowing() {
    let source = r#"
        function increment(value: number | undefined): number {
            if (value !== undefined) {
                return value + 1;
            }
            return 0;
        }
        function reversed(value: number | undefined): number {
            if (undefined !== value) {
                value = value + 2;
                return value ?? 0;
            }
            return -1;
        }
        function equality(value: string | undefined): string {
            if (value === undefined) {
                return "missing";
            } else {
                return value.toUpperCase();
            }
        }
        function negated(value: number | undefined): number {
            if (!(value === undefined)) {
                return value * 3;
            }
            return 2;
        }
        function guarded(value: number | undefined): number {
            if (value === undefined) return 10;
            return value + 4;
        }
        function guardedNegation(value: number | undefined): number {
            if (!(value !== undefined)) {
                return 20;
            }
            return value * 4;
        }
        function assigned(value: number | undefined): number {
            value = 5;
            return value + 1;
        }
        function reset(value: number | undefined): number {
            if (value === undefined) return 1;
            value = undefined;
            return value ?? 3;
        }
        function conjunction(value: number | undefined): number {
            if (value !== undefined && value > 2) {
                return value * 2;
            }
            return 0;
        }
        function disjunction(value: number | undefined): number {
            if (value === undefined || value > 5) {
                return 1;
            } else {
                return value + 10;
            }
        }
        function typeofGuard(value: number | undefined): number {
            if (typeof value !== "undefined") {
                return value + 5;
            }
            return 0;
        }
        function typeofElse(value: string | undefined): string {
            if ("undefined" === typeof value) {
                return "none";
            } else {
                return value.toUpperCase();
            }
        }
        async function delayed(value: number | undefined): Promise<number> {
            if (value !== undefined) {
                await sleep(1);
                return value * 2;
            }
            return 3;
        }
        async function main(): Promise<void> {
            console.log(increment(4));
            console.log(increment(undefined));
            console.log(reversed(5));
            console.log(reversed(undefined));
            console.log(equality("ok"));
            console.log(equality(undefined));
            console.log(negated(3));
            console.log(negated(undefined));
            console.log(guarded(6));
            console.log(guarded(undefined));
            console.log(guardedNegation(2));
            console.log(guardedNegation(undefined));
            console.log(assigned(undefined));
            console.log(reset(8));
            console.log(conjunction(4));
            console.log(conjunction(undefined));
            console.log(disjunction(3));
            console.log(disjunction(undefined));
            console.log(typeofGuard(2));
            console.log(typeofGuard(undefined));
            console.log(typeofElse("yes"));
            console.log(typeofElse(undefined));
            const optionalNumber: number | undefined = 1;
            const missingNumber: number | undefined = undefined;
            console.log(typeof optionalNumber);
            console.log(typeof missingNumber);
            console.log(await delayed(6));
            console.log(await delayed(undefined));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_branch_narrowing"),
        "5\n0\n7\n-1\nOK\nmissing\n9\n2\n10\n10\n8\n20\n6\n3\n8\n0\n13\n1\n7\n0\nYES\nnone\nnumber\nundefined\n12\n3\n"
    );
}

#[test]
fn compiles_optional_object_member_access() {
    let source = r#"
        interface Item {
            value: number;
            label: string | undefined;
        }
        function item(present: boolean): Item | undefined {
            if (present) return { value: 4, label: "ok" };
            return undefined;
        }
        async function delayed(present: boolean): Promise<Item | undefined> {
            await sleep(1);
            return item(present);
        }
        async function main(): Promise<void> {
            console.log(item(true)?.value);
            console.log(item(false)?.value);
            console.log(item(true)?.["value"]);
            console.log(item(true)?.label);
            console.log(item(false)?.label);
            console.log((await delayed(true))?.value);
            console.log((await delayed(false))?.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_object_member"),
        "4\nundefined\n4\nok\nundefined\n4\nundefined\n"
    );
}

#[test]
fn compiles_optional_record_and_json_member_access() {
    let source = r#"
        function record(present: boolean): Record<string, number> | undefined {
            if (present) return { value: 42 };
            return undefined;
        }
        function key(): string {
            console.log("key");
            return "value";
        }
        function json(present: boolean): Json | null {
            if (present) return JSON.parse("{\"name\":\"thaw\",\"items\":[10]}");
            return null;
        }
        async function delayed(): Promise<Record<string, number> | undefined> {
            await sleep(1);
            return { value: 7 };
        }
        async function main(): Promise<void> {
            console.log(record(true)?.value);
            console.log(record(false)?.value);
            console.log(record(true)?.[key()]);
            console.log(record(false)?.[key()]);
            console.log(String(json(true)?.name ?? JSON.parse("\"missing\"")));
            console.log(String(json(false)?.name ?? JSON.parse("\"missing\"")));
            console.log(Number(json(true)?.["items"]?.[0] ?? JSON.parse("0")));
            console.log((await delayed())?.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_record_json_member"),
        "42\nundefined\nkey\n42\nundefined\nthaw\nmissing\n10\n7\n"
    );
}

#[test]
fn compiles_optional_array_and_tuple_access() {
    let source = r#"
        function numbers(present: boolean): number[] | undefined {
            if (present) return [2, 4];
            return undefined;
        }
        function pair(present: boolean): [number, string] | undefined {
            if (present) return [3, "ok"];
            return undefined;
        }
        function text(present: boolean): string | undefined {
            if (present) return "thaw";
            return undefined;
        }
        function index(): number {
            console.log("index");
            return 1;
        }
        async function delayed(): Promise<number[] | undefined> {
            await sleep(1);
            return [6];
        }
        async function main(): Promise<void> {
            console.log(numbers(true)?.[index()]);
            console.log(numbers(false)?.[index()]);
            console.log(numbers(true)?.length);
            console.log(numbers(false)?.length);
            console.log(pair(true)?.[0]);
            console.log(pair(true)?.[1]);
            console.log(pair(true)?.length);
            console.log(pair(false)?.[0]);
            console.log(text(true)?.length);
            console.log(text(false)?.length);
            console.log((await delayed())?.[0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_array_tuple_access"),
        "index\n4\nundefined\n2\nundefined\n3\nok\n2\nundefined\n4\nundefined\n6\n"
    );
}

#[test]
fn compiles_optional_function_calls() {
    let source = r#"
        function callback(present: boolean): ((value: number) => number) | undefined {
            if (present) return (value: number) => value * 2;
            return undefined;
        }
        function action(present: boolean): ((value: number) => void) | undefined {
            if (present) return (value: number) => { console.log(value); };
            return undefined;
        }
        function argument(): number {
            console.log("argument");
            return 5;
        }
        async function delayedArgument(): Promise<number> {
            console.log("delayed argument");
            await sleep(1);
            return 6;
        }
        async function delayedCallback(): Promise<((value: number) => number) | undefined> {
            await sleep(1);
            return (value: number) => value + 1;
        }
        function spreadArguments(): [number] {
            console.log("spread arguments");
            return [9];
        }
        async function main(): Promise<void> {
            console.log(callback(true)?.(argument()));
            console.log(callback(false)?.(argument()));
            console.log(callback(true)?.(await delayedArgument()));
            console.log((await delayedCallback())?.(7));
            console.log(action(false)?.(argument()));
            console.log(action(true)?.(8));
            console.log(callback(false)?.(...spreadArguments()));
            console.log(callback(true)?.(...spreadArguments()));
            console.log(callback(true)?.(...[10]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_function_calls"),
        "argument\n10\nundefined\ndelayed argument\n12\n8\nundefined\n8\nundefined\nundefined\nspread arguments\n18\n20\n"
    );
}

#[test]
fn preserves_union_discriminants_through_nested_array_flattening() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        function main(): void {
            const number: Result = { kind: "number", value: 1 };
            const text: Result = { kind: "text", value: "flat" };
            const nested: Result[][] = [[number], [text]];
            const flattened = nested.slice().flat();
            print(flattened[0]);
            print(flattened[1]);
            const deep: Result[][][] = [nested];
            const flattenedDeep = deep.flat(2);
            print(flattenedDeep[0]);
            print(flattenedDeep[1]);
            const oneLevel = deep.flat();
            const flattenedAgain = oneLevel.flat();
            print(flattenedAgain[0]);
            print(flattenedAgain[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nested_union_array_flat"),
        "11\nflat!\n11\nflat!\n11\nflat!\n"
    );
}

#[test]
fn preserves_nested_union_metadata_through_function_returns() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        function syncValues(): Result[][] {
            const number: Result = { kind: "number", value: 1 };
            return [[number]];
        }
        async function asyncValues(): Promise<Result[][][]> {
            await sleep(1);
            const text: Result = { kind: "text", value: "returned" };
            return [[[text]]];
        }
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        async function main(): Promise<void> {
            const sync = syncValues().flat();
            print(sync[0]);
            const asynchronous = (await asyncValues()).flat(2);
            print(asynchronous[0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "returned_nested_union_array"),
        "11\nreturned!\n"
    );
}

#[test]
fn preserves_nested_union_metadata_through_function_values() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        function source(): Result[][] {
            const value: Result = { kind: "number", value: 1 };
            return [[value]];
        }
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        function main(): void {
            const annotated: () => Result[][] = () => {
                const value: Result = { kind: "text", value: "arrow" };
                return [[value]];
            };
            const arrowValues = annotated().flat();
            print(arrowValues[0]);
            const alias = source;
            const aliasedValues = alias().flat();
            print(aliasedValues[0]);
            const selected: () => Result[][] = true ? annotated : source;
            const selectedValues = selected().flat();
            print(selectedValues[0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "function_value_nested_union_array"),
        "arrow!\n11\narrow!\n"
    );
}

#[test]
fn preserves_nested_union_metadata_through_function_properties() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        interface Service {
            get(): Result;
            load(): Result[][];
            loadAsync(): Promise<Result[][]>;
        }
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        async function main(): Promise<void> {
            const service: Service = {
                get: (): Result => ({ kind: "text", value: "direct" }),
                load: (): Result[][] => [[{ kind: "number", value: 1 } as Result]],
                loadAsync: async (): Promise<Result[][]> =>
                    [[{ kind: "text", value: "async" } as Result]],
            };
            print(service.get());
            const load = service.load;
            print(load().flat()[0]);
            const { loadAsync: extractedAsync } = service;
            print((await extractedAsync()).flat()[0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "function_property_nested_union_array"),
        "direct!\n11\nasync!\n"
    );
}

#[test]
fn updates_union_metadata_when_reassigning_function_values() {
    let source = r#"
        type First =
            { kind: "a"; value: number } |
            { kind: "b"; value: string };
        type Second =
            { kind: "c"; value: number } |
            { kind: "d"; value: string };
        interface FirstService { load: () => First[][]; }
        interface SecondService { load: () => Second[][]; }
        function print(item: Second): void {
            if (item.kind === "c") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        function main(): void {
            const first: FirstService = {
                load: (): First[][] => [[{ kind: "a", value: 1 }]],
            };
            const second: SecondService = {
                load: (): Second[][] => [[{ kind: "c", value: 2 }]],
            };
            let load = first.load;
            load = second.load;
            print(load().flat()[0]);
            let destructuredLoad = first.load;
            ({ load: destructuredLoad } = second);
            print(destructuredLoad().flat()[0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "reassigned_function_union_metadata"),
        "12\n12\n"
    );
}

#[test]
fn updates_union_metadata_when_reassigning_function_properties() {
    let source = r#"
        type First =
            { kind: "a"; value: number } |
            { kind: "b"; value: string };
        type Second =
            { kind: "c"; value: number } |
            { kind: "d"; value: string };
        interface Service { load: () => First[][]; }
        interface Replacement { load: () => Second[][]; }
        interface Container { service: Service; }
        function print(item: Second): void {
            if (item.kind === "c") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        function main(): void {
            const service: Service = {
                load: (): First[][] => [[{ kind: "a", value: 1 }]],
            };
            const replacement: Replacement = {
                load: (): Second[][] => [[{ kind: "c", value: 2 }]],
            };
            service.load = replacement.load;
            print(service.load().flat()[0]);
            const nestedService: Service = {
                load: (): First[][] => [[{ kind: "b", value: "old" }]],
            };
            const container: Container = { service: nestedService };
            container.service.load = replacement.load;
            print(container.service.load().flat()[0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "reassigned_function_property_union_metadata"),
        "12\n12\n"
    );
}

#[test]
fn preserves_union_discriminants_for_array_construction_and_mapping() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        function identity(item: Result): Result { return item; }
        function expand(item: Result): Result[] { return [item]; }
        function print(values: Result[]): void {
            for (const item of values) {
                if (item.kind === "number") console.log(item.value + 10);
                else console.log(item.value + "!");
            }
        }
        function main(): void {
            const number: Result = { kind: "number", value: 1 };
            const text: Result = { kind: "text", value: "mapped" };
            const values = Array.of<Result>(number, text);
            print(values);
            print(Array.from<Result>(values));
            print(Array.from<Result, Result>(values, identity));
            print(values.map(identity));
            print(values.flatMap(expand));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "union_array_construction_metadata"),
        "11\nmapped!\n11\nmapped!\n11\nmapped!\n11\nmapped!\n11\nmapped!\n"
    );
}

#[test]
fn compiles_array_is_array_for_native_and_json_values() {
    let source = r#"
        function scalar(): number {
            console.log("scalar-evaluated");
            return 1;
        }
        async function delayed(): Promise<number[]> {
            await sleep(1);
            console.log("awaited-array");
            return [1, 2];
        }
        async function main(): Promise<void> {
            const numbers: number[] = [1, 2];
            const tuple: [number, string] = [1, "x"];
            console.log(Array.isArray(numbers));
            console.log(Array.isArray(tuple));
            console.log(Array.isArray({ x: 1 }));
            console.log(Array.isArray("text"));
            console.log(Array.isArray(JSON.parse("[1,2]")));
            console.log(Array.isArray(JSON.parse("{\"x\":1}")));
            console.log(Array.isArray(scalar()));
            console.log(Array.isArray(await delayed()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_is_array"),
        "true\ntrue\nfalse\nfalse\ntrue\nfalse\nscalar-evaluated\nfalse\nawaited-array\ntrue\n"
    );
}

#[test]
fn compiles_object_keys_for_fixed_objects() {
    let source = r#"
        interface Config { first: number; second: string; }
        function config(): Config {
            console.log("receiver");
            return { first: 1, second: "two" };
        }
        async function delayed(): Promise<Config> {
            console.log("awaited");
            await sleep(1);
            return { first: 3, second: "four" };
        }
        async function main(): Promise<void> {
            console.log(Object.keys(config()).join(","));
            console.log(Object.keys({ first: 1, second: 2, first: 3 }).join("-"));
            console.log(Object.keys(await delayed()).join("|"));
            console.log(Object.getOwnPropertyNames(config()).join("/"));
            console.log(Reflect.ownKeys(await delayed()).join("+"));
            const empty: {} = {};
            console.log(Object.keys(empty).length);
            const jsonObject: Json = JSON.parse("{\"second\":2,\"first\":1}");
            console.log(Object.keys(jsonObject).join(","));
            const record: Record<string, number> = { ten: 10, two: 2 };
            console.log(Object.keys(record).join(","));
            console.log(Object.getOwnPropertyNames(record).join("|"));
            console.log(Reflect.ownKeys(record).join("+"));
            console.log(Object.getOwnPropertyNames(JSON.parse("[10,20]")).join("|"));
            console.log(Reflect.ownKeys(JSON.parse("true")).length);
            const array: number[] = [10, 20, 30];
            console.log(Object.keys(array).join(","));
            const tuple: [number, string] = [1, "two"];
            console.log(Object.getOwnPropertyNames(tuple).join("|"));
            console.log(Reflect.ownKeys([]).length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_keys"),
        "receiver\nfirst,second\nfirst-second\nawaited\nfirst|second\nreceiver\nfirst/second\nawaited\nfirst+second\n0\nsecond,first\nten,two\nten|two\nten+two\n0|1\n0\n0,1,2\n0|1\n0\n"
    );
}

#[test]
fn compiles_object_has_own_for_fixed_objects() {
    let source = r#"
        interface Config { first: number; second: string; }
        function config(): Config {
            console.log("object");
            return { first: 1, second: "two" };
        }
        function key(): string {
            console.log("key");
            return "second";
        }
        async function delayedConfig(): Promise<Config> {
            console.log("awaited-object");
            await sleep(1);
            return { first: 3, second: "four" };
        }
        async function delayedKey(): Promise<string> {
            console.log("awaited-key");
            await sleep(1);
            return "first";
        }
        async function main(): Promise<void> {
            console.log(Object.hasOwn(config(), key()));
            console.log(Object.hasOwn({ first: 1 }, "missing"));
            console.log(Object.hasOwn({ "1": 1 }, 1));
            const empty: {} = {};
            console.log(Object.hasOwn(empty, "value"));
            console.log(Object.hasOwn(await delayedConfig(), await delayedKey()));
            const jsonObject: Json = JSON.parse("{\"value\":1}");
            console.log(Object.hasOwn(jsonObject, "value"));
            console.log(Object.hasOwn(jsonObject, "missing"));
            const record: Record<string, number> = { value: 1 };
            console.log(Object.hasOwn(record, "value"));
            console.log(Object.hasOwn(record, "missing"));
            console.log(Object.hasOwn(JSON.parse("[10,20]"), "1"));
            console.log(Object.hasOwn(JSON.parse("[10,20]"), "length"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_has_own"),
        "object\nkey\ntrue\nfalse\ntrue\nfalse\nawaited-object\nawaited-key\ntrue\ntrue\nfalse\ntrue\nfalse\ntrue\ntrue\n"
    );
}

#[test]
fn compiles_object_values_for_fixed_objects() {
    let source = r#"
        interface Config { first: number; second: string; enabled: boolean; }
        function config(): Config {
            console.log("receiver");
            return { first: 1, second: "two", enabled: true };
        }
        async function delayed(): Promise<{ left: number; right: number }> {
            console.log("awaited");
            await sleep(1);
            return { left: 3, right: 4 };
        }
        async function main(): Promise<void> {
            const mixed: [number, string, boolean] = Object.values(config());
            console.log(mixed.join("|"));
            const numbers: number[] = Object.values({ left: 1, right: 2 });
            console.log(numbers.join(","));
            console.log(Object.values(await delayed()).join("+"));
            const empty: {} = {};
            console.log(Object.values(empty).length);
            const jsonValues: Json[] = Object.values(JSON.parse("{\"second\":2,\"first\":\"one\"}"));
            console.log(Number(jsonValues[0]));
            console.log(String(jsonValues[1]));
            const numberRecord: Record<string, number> = { second: 2, first: 1 };
            const numberValues: number[] = Object.values(numberRecord);
            console.log(numberValues.join(","));
            const stringRecord: Record<string, string> = { left: "a", right: "b" };
            console.log(Object.values(stringRecord).join("+"));
            const boolRecord: Record<string, boolean> = { yes: true, no: false };
            console.log(Object.values(boolRecord).join("|"));
            const jsonRecord: Record<string, Json> = {
                first: JSON.parse("1"), second: JSON.parse("\"two\"")
            };
            const recordJsonValues: Json[] = Object.values(jsonRecord);
            console.log(Number(recordJsonValues[0]));
            console.log(String(recordJsonValues[1]));
            console.log(Object.values(JSON.parse("[true,false]")).length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_values"),
        "receiver\n1|two|true\n1,2\nawaited\n3+4\n0\n2\none\n2,1\na+b\ntrue|false\n1\ntwo\n2\n"
    );
}

#[test]
fn compiles_object_entries_for_fixed_objects() {
    let source = r#"
        interface Config { first: number; second: string; enabled: boolean; }
        function config(): Config {
            console.log("receiver");
            return { first: 1, second: "two", enabled: true };
        }
        async function delayed(): Promise<{ left: number; right: number }> {
            console.log("awaited");
            await sleep(1);
            return { left: 3, right: 4 };
        }
        async function main(): Promise<void> {
            const mixed: [[string, number], [string, string], [string, boolean]] = Object.entries(config());
            console.log(mixed[0][0]);
            console.log(mixed[0][1]);
            console.log(mixed[1][0]);
            console.log(mixed[1][1]);
            console.log(mixed[2][0]);
            console.log(mixed[2][1]);
            const numeric: [string, number][] = Object.entries({ left: 1, right: 2 });
            console.log(numeric[1][0]);
            console.log(numeric[1][1]);
            const awaited: [string, number][] = Object.entries(await delayed());
            console.log(awaited[0][0]);
            console.log(awaited[0][1]);
            const empty: {} = {};
            console.log(Object.entries(empty).length);
            const jsonEntries: [string, Json][] = Object.entries(JSON.parse("{\"second\":2,\"first\":\"one\"}"));
            console.log(jsonEntries[0][0]);
            console.log(Number(jsonEntries[0][1]));
            console.log(jsonEntries[1][0]);
            console.log(String(jsonEntries[1][1]));
            const numberRecord: Record<string, number> = { second: 2, first: 1 };
            const numberEntries: [string, number][] = Object.entries(numberRecord);
            console.log(numberEntries[0][0]);
            console.log(numberEntries[0][1]);
            const stringRecord: Record<string, string> = { left: "a", right: "b" };
            const stringEntries: [string, string][] = Object.entries(stringRecord);
            console.log(stringEntries[1][0]);
            console.log(stringEntries[1][1]);
            const boolRecord: Record<string, boolean> = { yes: true, no: false };
            const boolEntries: [string, boolean][] = Object.entries(boolRecord);
            console.log(boolEntries[1][0]);
            console.log(boolEntries[1][1]);
            const jsonRecord: Record<string, Json> = {
                first: JSON.parse("1"), second: JSON.parse("\"two\"")
            };
            const recordJsonEntries: [string, Json][] = Object.entries(jsonRecord);
            console.log(recordJsonEntries[0][0]);
            console.log(Number(recordJsonEntries[0][1]));
            console.log(recordJsonEntries[1][0]);
            console.log(String(recordJsonEntries[1][1]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_entries"),
        "receiver\nfirst\n1\nsecond\ntwo\nenabled\ntrue\nright\n2\nawaited\nleft\n3\n0\nsecond\n2\nfirst\none\nsecond\n2\nright\nb\nno\nfalse\nfirst\n1\nsecond\ntwo\n"
    );
}

#[test]
fn compiles_object_from_typed_entries() {
    let source = r#"
        function numberEntries(): [string, number][] {
            console.log("entries");
            return [["first", 1], ["second", 2], ["first", 3]];
        }
        async function stringEntries(): Promise<[string, string][]> {
            await sleep(1);
            console.log("awaited");
            return [["left", "a"], ["right", "b"]];
        }
        async function main(): Promise<void> {
            const numbers: Record<string, number> = Object.fromEntries(numberEntries());
            console.log(Object.keys(numbers).join(","));
            console.log(numbers.first);
            console.log(numbers.second);
            const strings: Record<string, string> = Object.fromEntries(await stringEntries());
            console.log(strings.left + strings.right);
            const booleans: Record<string, boolean> = Object.fromEntries([
                ["yes", true], ["no", false]
            ] as [string, boolean][]);
            console.log(booleans.yes);
            console.log(booleans.no);
            const jsonEntries: [string, Json][] = [["value", JSON.parse("42")]];
            const jsonValues: Record<string, Json> = Object.fromEntries(jsonEntries);
            console.log(Number(jsonValues.value));
            const roundTrip: Record<string, number> = Object.fromEntries(Object.entries(numbers));
            console.log(roundTrip.first);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_from_entries"),
        "entries\nfirst,second\n3\n2\nawaited\nab\ntrue\nfalse\n42\n3\n"
    );
}

#[test]
fn compiles_object_assign_for_runtime_keyed_objects() {
    let source = r#"
        function source(label: string, shared: number): Record<string, number> {
            console.log(label);
            return { shared, second: 2 };
        }
        async function delayed(): Promise<Record<string, number>> {
            await sleep(1);
            console.log("awaited");
            return { shared: 3, third: 4 };
        }
        async function main(): Promise<void> {
            const target: Record<string, number> = { first: 1, shared: 1 };
            const result: Record<string, number> = Object.assign(
                target, source("source", 2), await delayed()
            );
            console.log(Object.keys(result).join(","));
            console.log(result.shared);
            console.log(result.second);
            console.log(result.third);
            console.log(target.shared);
            console.log(Object.assign(target).first);
            const jsonTarget: Json = JSON.parse("{\"first\":1}");
            const jsonResult: Json = Object.assign(
                jsonTarget, JSON.parse("{\"second\":2}")
            );
            console.log(JSON.stringify(jsonResult));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_assign"),
        "source\nawaited\nfirst,shared,second,third\n3\n2\n4\n3\n1\n{\"first\":1,\"second\":2}\n"
    );
}

#[test]
fn compiles_object_is_same_value_comparisons() {
    let source = r#"
        interface Item { value: number; }
        function text(): string { return "same"; }
        function left(): number {
            console.log("left");
            return 1;
        }
        async function right(): Promise<number> {
            console.log("right");
            await sleep(1);
            return 1;
        }
        async function pending(): Promise<number> {
            await sleep(1);
            return 1;
        }
        async function main(): Promise<void> {
            console.log(Object.is(0 / 0, 0 / 0));
            console.log(Object.is(0, -0));
            console.log(Object.is(-0, -0));
            console.log(Object.is(1, 1));
            console.log(Object.is("same", text()));
            console.log(Object.is(true, true));
            console.log(Object.is(1, "1"));
            const item: Item = { value: 1 };
            console.log(Object.is(item, item));
            console.log(Object.is(item, { value: 1 }));
            const array: number[] = [1];
            console.log(Object.is(array, array));
            console.log(Object.is(array, [1]));
            const promise: Promise<number> = pending();
            console.log(Object.is(promise, promise));
            console.log(Object.is(promise, pending()));
            console.log(Object.is(left(), await right()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_is"),
        "true\nfalse\ntrue\ntrue\ntrue\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse\nleft\nright\ntrue\n"
    );
}

#[test]
fn compiles_tuple_spreads_for_reflection_builtins() {
    let source = r#"
        function main(): void {
            console.log(Array.isArray(...[[1, 2]]));
            console.log(Object.keys(...[{ first: 1, second: 2 }]).join(","));
            console.log(Object.getOwnPropertyNames(...[{ first: 1 }]).join(","));
            console.log(Reflect.ownKeys(...[{ first: 1 }]).join(","));
            console.log(Object.values(...[{ first: 1, second: 2 }]).join(","));
            console.log(Object.entries(...[{ first: 1, second: 2 }]).length);
            console.log(Object.hasOwn(...[{ first: 1 }, "first"]));
            console.log(Object.is(...[NaN, NaN]));
            const jsonNumber: Json = JSON.parse("1");
            const jsonObject: Json = JSON.parse("{\"value\":1}");
            console.log(Object.is(jsonNumber, JSON.parse("1")));
            console.log(Object.is(jsonNumber, 1));
            console.log(Object.is("thaw", JSON.parse("\"thaw\"")));
            console.log(Object.is(JSON.parse("true"), true));
            console.log(Object.is(jsonObject, jsonObject));
            console.log(Object.is(jsonObject, JSON.parse("{\"value\":1}")));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "reflection_builtin_tuple_spreads"),
        "true\nfirst,second\nfirst\nfirst\n1,2\n2\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\n"
    );
}

#[test]
fn compiles_object_literals_field_access_and_mutation() {
    let source = r#"
        function dist(p: { x: number; y: number }): number {
            return p.x + p.y;
        }

        function main(): void {
            const p: { x: number; y: number } = { y: 2, x: 1 };
            console.log(p.x);
            console.log(dist(p));

            p.x = p.x + 10;
            console.log(p.x);

            console.log(dist({ x: 3, y: 4 }));
        }
    "#;
    assert_eq!(compile_and_run(source, "objects"), "1\n3\n11\n7\n");
}

#[test]
fn runs_call_result_object_spread_once() {
    let source = r#"
        function makeConfig(): { x: number; label: string } {
            console.log("make");
            return { x: 7, label: "base" };
        }
        function main(): void {
            const point: { x: number; label: string } = {
                ...makeConfig(),
                label: "point"
            };
            console.log(point.x);
            console.log(point.label);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "call_result_object_spread"),
        "make\n7\npoint\n"
    );
}

#[test]
fn runs_multiple_object_spreads_once_in_source_order() {
    let source = r#"
        function makeX(): { x: number } {
            console.log("left");
            return { x: 7 };
        }
        function makeLabel(): { label: string } {
            console.log("right");
            return { label: "point" };
        }
        function main(): void {
            const point: { x: number; label: string } = {
                ...makeX(),
                ...makeLabel()
            };
            console.log(point.x);
            console.log(point.label);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "multiple_object_spreads"),
        "left\nright\n7\npoint\n"
    );
}

#[test]
fn runs_conditional_object_spread_once() {
    let source = r#"
        function left(): { value: number } {
            console.log("left");
            return { value: 7 };
        }
        function right(): { value: number } {
            console.log("right");
            return { value: 9 };
        }
        function main(): void {
            const chooseLeft: boolean = true;
            const config: { value: number } = {
                ...(chooseLeft ? left() : right())
            };
            console.log(config.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "conditional_object_spread"),
        "left\n7\n"
    );
}

#[test]
fn compiles_nested_object_fields() {
    let source = r#"
        interface Point {
            x: number;
            y: number;
        }
        interface Line {
            start: Point;
            length: number;
        }

        function main(): void {
            const l: Line = { length: 5, start: { x: 1, y: 2 } };
            console.log(l.start.x);
            console.log(l.start.y);
            console.log(l.length);

            l.start.x = l.start.x + 100;
            console.log(l.start.x);

            l.start = { x: 9, y: 9 };
            console.log(l.start.x);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nested_objects"),
        "1\n2\n5\n101\n9\n"
    );
}

#[test]
fn executes_top_level_destructuring_once() {
    let source = r#"
        const { point: { x, y }, values: [first, second] } = {
            point: { x: 40, y: 2 },
            values: [20, 22]
        };
        function main(): void {
            console.log(x + y);
            console.log(first + second);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "top_level_destructuring"),
        "42\n42\n"
    );
}

#[test]
fn executes_top_level_destructuring_defaults_and_array_rest() {
    let source = r#"
        interface Config { fallback: number | undefined; }
        const { fallback = 42 }: Config = { fallback: undefined };
        const nullable: [number | null] = [null];
        const [keptNull = topFallback()] = nullable;
        const nullish: [number | null | undefined, number | null | undefined] =
            [null, undefined];
        const [alsoNull = topFallback(), defaultedUndefined = topFallback()] = nullish;
        const [head, ...tail] = [20, 10, 12];
        const { answer, ...metadata } = { answer: 42, label: "ready", code: 2 };
        function topFallback(): number {
            console.log("top fallback");
            return 7;
        }
        function main(): void {
            console.log(fallback);
            console.log(keptNull);
            console.log(alsoNull);
            console.log(defaultedUndefined);
            console.log(head + tail[0] + tail[1]);
            console.log(metadata.label);
            console.log(answer + metadata.code - 2);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "top_level_destructuring_default_rest"),
        "top fallback\n42\nnull\nnull\n7\n42\nready\n42\n"
    );
}

#[test]
fn allocates_object_identity_before_field_assignment() {
    let object_type = HirType::Object(vec![
        ("value".into(), HirType::F64),
        ("label".into(), HirType::Str),
    ]);
    let object = HirExpr::Var("instance".into());
    let program = HirProgram {
        functions: vec![HirFunction {
            name: "main".into(),
            params: Vec::new(),
            ret: HirType::Void,
            is_async: false,
            body: vec![
                HirStmt::Let(
                    "instance".into(),
                    object_type.clone(),
                    HirExpr::ObjectAlloc(object_type.clone()),
                ),
                HirStmt::Expr(HirExpr::PropAssign(
                    Box::new(object.clone()),
                    object_type.clone(),
                    "value".into(),
                    Box::new(HirExpr::Lit(HirLit::F64(42.0))),
                )),
                HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("console.log".into())),
                    vec![HirExpr::PropAccess(
                        Box::new(object),
                        object_type,
                        "value".into(),
                    )],
                )),
                HirStmt::Return(None),
            ],
        }],
        ..HirProgram::default()
    };
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "object_identity_allocation");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("%object_alloc = call ptr @thaw_arena_alloc"));
    assert!(ir.contains("%object_zero_field"));
    assert!(ir.contains("store double 4.200000e+01"));
}

#[test]
fn compiles_and_runs_top_level_default_and_optional_parameters() {
    let source = r#"
        function describe(
            prefix: string = "value=",
            value?: number,
            suffix: string = "!"
        ): string {
            return prefix + String(value ?? 5) + suffix;
        }
        async function delayed(value: number = 20): Promise<number> {
            await sleep(1);
            return value + 22;
        }
        async function main(): Promise<void> {
            console.log(describe());
            console.log(describe("n=", 7));
            console.log(describe(undefined, 8, "?"));
            console.log(describe("empty=", undefined, "."));
            console.log(await delayed());
            console.log(await delayed(undefined));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "top_level_default_optional_parameters"),
        "value=5!\nn=7!\nvalue=8?\nempty=5.\n42\n42\n"
    );
}

#[test]
fn frame_split_supports_fixed_object_for_in() {
    let source = r#"
        function source() {
            console.log("source");
            return { first: 1, skip: 2, last: 3 };
        }
        async function main(): Promise<void> {
            for (const key in source()) {
                await sleep(1);
                if (key === "skip") continue;
                console.log(key);
                if (key === "last") break;
            }
            let retained = "";
            for (retained in { alpha: 1, omega: 2 }) {}
            console.log(retained);
            const dynamic: Json = JSON.parse("{\"second\":2,\"first\":1}");
            for (const key in dynamic) console.log(key);
            const record: Record<string, number> = { ten: 10, two: 2 };
            for (const key in record) console.log(key);
            for (const index in JSON.parse("[10,20]")) console.log(index);
            for (const value of JSON.parse("[1,\"two\",true]")) console.log(String(value));
            for (const character of "A😀é") console.log(character);
            for await (const character of "雪a") console.log(character);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_for_in"),
        "source\nfirst\nlast\nomega\nsecond\nfirst\nten\ntwo\n0\n1\n1\ntwo\ntrue\nA\n😀\né\n雪\na\n"
    );
}
