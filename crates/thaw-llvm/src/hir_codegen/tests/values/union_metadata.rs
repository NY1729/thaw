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

