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
fn compiles_array_push_pop_shift_unshift_with_reference_sharing() {
    let source = r#"
        async function main(): Promise<void> {
            const a: number[] = [1, 2, 3];
            const alias = a;
            console.log(a.push(4));
            console.log(a.push(5, 6));
            console.log(a.join(","), alias.join(","), a === alias);
            console.log(a.pop());
            console.log(a.join(","), alias.join(","));
            const words: string[] = ["y", "z"];
            const wordsAlias = words;
            console.log(words.unshift("w", "x"));
            console.log(words.join(""), wordsAlias.join(""));
            console.log(words.shift());
            console.log(words.join(""), wordsAlias.join(""));
            const empty: number[] = [];
            console.log(empty.pop(), empty.shift(), empty.length);
            const nested: number[][] = [];
            const poppedNested = nested.pop();
            console.log(poppedNested.length);
            const single: number[] = [42];
            console.log(single.push());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_push_pop_shift_unshift"),
        concat!(
            "4\n",
            "6\n",
            "1,2,3,4,5,6 1,2,3,4,5,6 true\n",
            "6\n",
            "1,2,3,4,5 1,2,3,4,5\n",
            "4\n",
            "wxyz wxyz\n",
            "w\n",
            "xyz xyz\n",
            "0 0 0\n",
            "0\n",
            "1\n",
        )
    );
}

#[test]
fn compiles_array_splice_removes_inserts_and_shares_the_receiver() {
    let source = r#"
        async function main(): Promise<void> {
            const a: number[] = [1, 2, 3, 4, 5];
            const alias = a;
            const removed = a.splice(1, 2, 10, 11, 12);
            console.log(removed.join(","), a.join(","), alias.join(","));
            const b: number[] = [1, 2, 3, 4, 5];
            const tail = b.splice(2);
            console.log(tail.join(","), b.join(","));
            const c: string[] = ["a", "b", "c"];
            const none = c.splice(1, 0, "x");
            console.log(none.length, c.join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_splice"),
        concat!(
            "2,3 1,10,11,12,4,5 1,10,11,12,4,5\n",
            "3,4,5 1,2\n",
            "0 a,x,b,c\n",
        )
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

