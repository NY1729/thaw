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
        "receiver\nfirst,second\nfirst-second\nawaited\nfirst|second\nreceiver\nfirst/second\nawaited\nfirst+second\n0\nsecond,first\nten,two\nten|two\nten+two\n0|1|length\n0\n0,1,2\n0|1|length\n1\n"
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
fn observes_fixed_object_accessors_across_value_consumers() {
    let source = r#"
        let calls = 0;
        let order = "";
        function fresh() {
            console.log("receiver");
            return { get value(): number { calls++; return 42; } };
        }
        function main(): void {
            const source = {
                base: 40,
                get value(): number { calls++; order += "v"; return this.base + 2; },
            };
            console.log(source["value"], source?.["value"], calls);
            const { value } = source;
            console.log(value, calls);
            const union = true ? source : { base: 0, get value(): number { return 0; } };
            console.log(union.value, calls);

            const spread = { ...source };
            const { base, ...rest } = source;
            source.base = 1;
            console.log(spread.value, rest.value, base, calls);
            console.log(Object.values(source).join("|"), calls);
            console.log(Object.entries(source)[1][1], calls);
            console.log(Object.keys(source).join(","), calls);
            console.log(Object.getOwnPropertyNames(source).join(","), calls);

            const nested = { source };
            console.log(JSON.stringify(nested), calls);
            const clone = structuredClone(source);
            source.base = 100;
            console.log(clone.value, source.value, calls);

            const writeOnly = { set hidden(next: number) { calls += next; } };
            console.log(Object.keys(writeOnly).join(","));
            console.log(Object.values(writeOnly)[0]);
            console.log(JSON.stringify(writeOnly));
            console.log(Object.keys(spread).join(","));
            console.log(fresh()["value"], calls);
            console.log(order);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "fixed_object_accessor_observations"),
        "42 42 2\n42 3\n42 4\n42 42 40 6\n1|3 7\n3 8\nbase,value 8\nbase,value 8\n{\"source\":{\"base\":1,\"value\":3}} 9\n3 102 11\nhidden\nundefined\n{}\nbase,value\nreceiver\n42 12\nvvvvvvvvvvv\n"
    );
}

#[test]
fn observes_accessor_key_order_replacers_and_overwrites() {
    let source = r#"
        let order = "";
        function observed(label: string, value: number): number {
            order += label;
            return value;
        }
        function main(): void {
            const source = {
                z: 0,
                get "10"(): number { return observed("10,", 10); },
                get "2"(): number { return observed("2,", 2); },
                get a(): number { return observed("a,", 1); },
            };
            console.log(Object.keys(source).join("|"), order);
            console.log(Object.values(source).join("|"), order);
            order = "";
            const selected = JSON.stringify(source, ["a", "2"]);
            console.log(selected, order);
            const overwritten = {
                get value(): number { order += "bad"; return 1; },
                value: 42,
            };
            console.log(overwritten.value, overwritten["value"], JSON.stringify(overwritten));
            try {
                const throwing = {
                    get value(): number { throw new Error("boom"); return 0; }
                };
                console.log(throwing.value);
            } catch (error) {
                console.log(error.message);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "accessor_order_replacers_overwrites"),
        "2|10|z|a \n2|10|0|1 2,10,a,\n{\"a\":1,\"2\":2} a,2,\n42 42 {\"value\":42}\nboom\n"
    );
}

#[test]
fn observes_native_accessor_descriptors_and_json_replacers() {
    let source = r#"
        let stored = 2;
        let order = "";
        let reads = 0;
        function main(): void {
            const source = {
                get a(): number { reads++; order += "ga,"; return stored; },
                set a(value: number) { order += "sa,"; stored = value; },
                get b(): number { reads++; order += "gb,"; return 3; },
            };
            const descriptor = Object.getOwnPropertyDescriptor(source, "a")!;
            console.log(typeof descriptor.get, typeof descriptor.set,
                descriptor.enumerable, descriptor.configurable,
                Object.hasOwn(descriptor, "value"));
            console.log(descriptor.get.call(source));
            descriptor.set.call(source, 7);
            console.log(source.a, order);
            const descriptors = Object.getOwnPropertyDescriptors(source);
            console.log(typeof descriptors.a.get, typeof descriptors.b.get,
                Object.keys(descriptors).join(","));

            order = "";
            const keys: string[] = ["b", "a", "b"];
            const selected = JSON.stringify(source, keys);
            console.log(selected, order);

            order = "";
            const readsBeforeReplacer = reads;
            const replaced = JSON.stringify(source, (key: string, value: any): any => {
                return key === "b" ? undefined : value;
            });
            console.log(replaced, reads - readsBeforeReplacer);

            const nested = { source };
            const shallowKeys: string[] = ["source"];
            const nestedKeys: string[] = ["source", "a"];
            const readsBeforeNested = reads;
            console.log(JSON.stringify(nested, shallowKeys), reads - readsBeforeNested);
            console.log(JSON.stringify(nested, nestedKeys), reads - readsBeforeNested);

            const absent = {
                get missing(): undefined { reads++; return undefined; },
                set writeOnly(value: number) { stored = value; },
            };
            const absentKeys: string[] = ["missing", "writeOnly"];
            const absentJson = JSON.stringify(absent, absentKeys);
            const writeDescriptor = Object.getOwnPropertyDescriptor(absent, "writeOnly")!;
            console.log(absentJson, reads, writeDescriptor.get === undefined,
                typeof writeDescriptor.set);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_accessor_descriptors_replacers"),
        "function function true true false\n2\n7 ga,sa,ga,\nfunction function a,b\n{\"b\":3,\"a\":7} gb,ga,\n{\"a\":7} 2\n{\"source\":{}} 0\n{\"source\":{\"a\":7}} 1\n{} 8 true function\n"
    );
}

#[test]
fn redefines_existing_native_accessors_from_descriptors() {
    let source = r#"
        function main(): void {
            const source = {
                base: 1,
                get value(): number { return this.base; },
                set value(next: number) { this.base = next; },
            };
            const same = Object.defineProperty(source, "value", {
                get(): number { return this.base + 10; },
                set(next: number) { this.base = next * 2; },
            });
            console.log(same === source, source.value);
            source.value = 3;
            console.log(source.base, source.value);

            console.log(Reflect.defineProperty(source, "value", {
                get: (): number => source.base + 20,
                set: (next: number): void => { source.base = next + 1; },
            }));
            console.log(source.value);
            source.value = 4;
            console.log(source.base, source.value);

            Object.defineProperties(source, {
                value: {
                    get(): number { return this.base + 30; },
                    set(next: number) { this.base = next + 2; },
                },
            });
            source.value = 8;
            console.log(source.base, source.value);
            const descriptor = Object.getOwnPropertyDescriptor(source, "value")!;
            console.log(descriptor.get.call(source));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_accessor_descriptor_redefinition"),
        "true 11\n6 16\ntrue\n26\n5 25\n10 40\n40\n"
    );
}

#[test]
fn rejects_fixed_layout_descriptor_shape_changes() {
    for (source, expected) in [
        (
            r#"function main(): void {
                const source = { value: 1 };
                Object.defineProperty(source, "value", { get: (): number => 2 });
            }"#,
            "cannot add an accessor slot to fixed field `value`",
        ),
        (
            r#"function main(): void {
                const source = { get value(): number { return 1; } };
                Object.defineProperty(source, "value", { value: 2 });
            }"#,
            "cannot convert accessor `value` to a data property on a fixed object",
        ),
        (
            r#"function main(): void {
                const source = { get value(): number { return 1; } };
                Object.defineProperty(source, "value", { set: (next: number): void => {} });
            }"#,
            "cannot add an accessor slot to fixed field `value`",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = thaw_hir::lower_module(&module).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
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
fn object_from_entries_consumes_compatible_union_arrays_once() {
    let source = r#"
        let calls = 0;
        function entries(numbers: boolean): [string, number][] | [string, string][] {
            calls++;
            const numberFirst: [string, number] = ["first", 1];
            const numberSecond: [string, number] = ["second", 2];
            if (numbers) return [numberFirst, numberSecond];
            const stringFirst: [string, string] = ["first", "a"];
            const stringSecond: [string, string] = ["second", "b"];
            return [stringFirst, stringSecond];
        }
        function main(): void {
            console.log(JSON.stringify(Object.fromEntries(entries(true))));
            console.log(JSON.stringify(Object.fromEntries(entries(false))));
            console.log(calls);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_from_entries_union_arrays"),
        "{\"first\":1,\"second\":2}\n{\"first\":\"a\",\"second\":\"b\"}\n2\n"
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
fn compiles_object_assign_for_plain_object_literals() {
    let source = r#"
        function main(): void {
            console.log(Object.assign({}, { a: 1 }, { b: 2 }));
            console.log(Object.assign({ a: 1 }, { b: 2 }, { c: 3 }));
            console.log(Object.fromEntries([["a", 1], ["b", 2]]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_assign_plain_literals"),
        "{\"a\":1,\"b\":2}\n{\"a\":1,\"b\":2,\"c\":3}\n{\"a\":1,\"b\":2}\n"
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
