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
fn tuple_join_uses_empty_text_for_only_absent_slots() {
    let source = r#"
        let calls = 0;
        function make(): [number | null | undefined, string, string, number | undefined] {
            calls++;
            return [null, "null", "undefined", undefined];
        }
        function main(): void {
            const staticSlots: [null, undefined, string, string] =
                [null, undefined, "null", "undefined"];
            console.log(String(staticSlots));
            console.log(staticSlots.join("|"));
            console.log(staticSlots.join());
            const optional: [number | undefined, number | undefined] = [undefined, 4];
            const nullable: [number | null, number | null] = [null, 5];
            const nullish: [number | null | undefined, number | null | undefined, number | null | undefined] =
                [null, undefined, 6];
            console.log(String(optional), optional.join("|"));
            console.log(String(nullable), nullable.join("|"));
            console.log(String(nullish), nullish.join("|"));
            const union: [number | string | null | undefined, number | string | null | undefined,
                          number | string | null | undefined, number | string | null | undefined] =
                [null, undefined, "null", 7];
            console.log(String(union), union.join("|"));
            console.log(String(make()), calls);
            console.log(make().join("|"), calls);
            console.log(String(null), String(undefined));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tuple_join_absent_static_tagged_union"),
        ",,null,undefined\n||null|undefined\n,,null,undefined\n,4 |4\n,5 |5\n,,6 ||6\n,,null,7 ||null|7\n,null,undefined, 1\n|null|undefined| 2\nnull undefined\n"
    );
}

#[test]
fn tuple_join_checks_json_and_live_jsvalue_slots_without_stringifying_absence() {
    let source = r#"
        function main(): void {
            loadScript("globalThis.joinNull = null; globalThis.joinMissing = undefined; globalThis.joinText = 'null'; globalThis.joinObject = { toString() { return 'object'; }, toJSON() { throw new Error('snapshot'); } }; globalThis.joinObject.self = globalThis.joinObject;");
            const jsonNull: any = JSON.parse("null");
            const jsonMissing: any = undefined;
            const jsonText: any = JSON.parse("\"undefined\"");
            const jsonSlots: [any, any, any] = [jsonNull, jsonMissing, jsonText];
            console.log(String(jsonSlots), jsonSlots.join("|"));
            const liveNull: JsValue = getDynamicValue("joinNull");
            const liveMissing: JsValue = getDynamicValue("joinMissing");
            const liveText: JsValue = getDynamicValue("joinText");
            const liveObject: JsValue = getDynamicValue("joinObject");
            const liveSlots: [JsValue, JsValue, JsValue, JsValue] =
                [liveNull, liveMissing, liveText, liveObject];
            console.log(String(liveSlots), liveSlots.join("|"));
            console.log(String(jsonNull), String(jsonMissing));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tuple_join_absent_json_live"),
        ",,undefined ||undefined\n,,null,object ||null|object\nnull undefined\n"
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

/// A tuple-valued index must be captured before its elements are stringified.
/// The empty tuple still evaluates its source once despite using no elements.
#[test]
fn tuple_index_string_conversion_evaluates_source_once() {
    let source = r#"
        let makes = 0;
        let indexes = 0;
        let empties = 0;
        function make(): [number, string][] {
            makes++;
            return [[1, "x"]];
        }
        function index(): number {
            indexes++;
            return 0;
        }
        function empty(): [[]] {
            empties++;
            return [[]];
        }
        function main(): void {
            const text = String(make()[index()]);
            console.log(text === "1,x" && makes === 1 && indexes === 1);
            const blank = String(empty()[0]);
            console.log(blank === "" && empties === 1);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tuple_index_string_conversion_once"),
        "true\ntrue\n"
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
            console.log(poppedNested === undefined);
            const leadingHole: number[] = [, 7];
            console.log(leadingHole.shift(), leadingHole.length, 0 in leadingHole);
            const trailingHole: number[] = [7, ,];
            console.log(trailingHole.pop(), trailingHole.pop(), trailingHole.length, 0 in trailingHole);
            const single: number[] = [42];
            console.log(single.push());
            const tagged: (number | undefined)[] = [undefined, 2];
            console.log(tagged.shift() === undefined, tagged.pop() === 2, tagged.pop() === undefined);
            const taggedSingle: (number | undefined)[] = [undefined];
            console.log(taggedSingle.pop() === undefined);
            const nullish: (number | null | undefined)[] = [null, undefined, 3];
            console.log(nullish.shift() === null, nullish.shift() === undefined, nullish.pop(), nullish.pop() === undefined);
            const onlyUndefined = [undefined];
            console.log(onlyUndefined.pop() === undefined, onlyUndefined.pop() === undefined);
            const union: (number | string | undefined)[] = [undefined, 3];
            console.log(union.shift() === undefined, union.pop(), union.pop() === undefined);
            const taggedSource: (number | undefined)[] = [, 1];
            const copiedTagged = taggedSource.toReversed();
            console.log(copiedTagged.pop() === undefined, copiedTagged.shift() === 1);
            const nullable: (number | null)[] = [null, 3];
            console.log(nullable.shift() === null, nullable.pop(), nullable.pop() === undefined);
            const unionNull: (number | string | null)[] = [null];
            console.log(unionNull.pop() === null, unionNull.pop() === undefined);
            const onlyNull = [null];
            console.log(onlyNull.pop() === null, onlyNull.pop() === undefined);
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
            "undefined undefined 0\n",
            "true\n",
            "undefined 1 true\n",
            "undefined 7 0 false\n",
            "1\n",
            "true true true\n",
            "true\n",
            "true true 3 true\n",
            "true true\n",
            "true 3 true\n",
            "true true\n",
            "true 3 true\n",
            "true true\n",
            "true true\n",
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

#[test]
fn compiles_mutating_methods_on_unions_of_arrays() {
    let source = r#"
        let receiverCalls = 0;
        let argumentCalls = 0;
        function values(text: boolean): number[] | string[] {
            receiverCalls++;
            if (text) {
                const result: string[] = ["c", , "a"];
                return result;
            }
            const result: number[] = [3, , 1];
            return result;
        }
        function index(value: number): number {
            argumentCalls++;
            return value;
        }
        function insertable(wide: boolean): number[] | (number | string)[] {
            if (wide) {
                const result: (number | string)[] = [1, "x"];
                return result;
            }
            const result: number[] = [2];
            return result;
        }
        function compare(left: number | string, right: number | string): number {
            console.log("compare");
            return 0;
        }
        function comparator(): (left: number | string, right: number | string) => number {
            console.log("comparator");
            return compare;
        }
        function main(): void {
            for (const text of [false, true]) {
                const copied = values(text);
                console.log(copied.copyWithin(index(0), index(1)) === copied, copied.join(","));
                const reversed = values(text);
                console.log(reversed.reverse() === reversed, reversed.join(","));
                const sorted = values(text);
                console.log(sorted.sort(comparator()) === sorted, sorted.join(","));
                const popped = values(text);
                console.log(popped.pop(), popped.join(","));
                const shifted = values(text);
                console.log(shifted.shift(), shifted.join(","));
                const spliced = values(text);
                const removed = spliced.splice(index(1), index(1));
                console.log(removed.join(","), spliced.join(","));
            }
            const inserted = insertable(false);
            console.log(inserted.push(index(4)), inserted.unshift(index(0)), inserted.join(","));
            const filled = insertable(true);
            console.log(filled.fill(index(7), index(1)) === filled, filled.join(","));
            console.log(receiverCalls, argumentCalls);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "union_array_mutations"),
        concat!(
            "true ,1,1\n",
            "true 1,,3\n",
            "comparator\ncompare\ntrue 3,1,\n",
            "1 3,\n",
            "3 ,1\n",
            " 3,1\n",
            "true ,a,a\n",
            "true a,,c\n",
            "comparator\ncompare\ntrue c,a,\n",
            "a c,\n",
            "c ,a\n",
            " c,a\n",
            "2 3 0,2,4\n",
            "true 1,7\n",
            "12 12\n",
        )
    );
}

#[test]
fn destructures_union_arrays_with_defaults_rest_and_single_evaluation() {
    let source = r#"
        let calls = 0;
        function values(numbers: boolean): number[] | string[] {
            calls++;
            if (numbers) return [1, , 3, 4];
            return ["a", , "c", "d"];
        }
        function main(): void {
            const [numberFirst, numberDefault = 2, ...numberRest] = values(true);
            console.log(String(numberFirst), String(numberDefault), numberRest.join("|"));
            const [wordFirst, wordDefault = "b", ...wordRest] = values(false);
            console.log(String(wordFirst), String(wordDefault), wordRest.join("|"));

            let first: number | string | undefined = 0;
            let second: number | string = 0;
            let rest: (number | string)[] = [];
            [first, second = 2, ...rest] = values(true);
            console.log(String(first), String(second), rest.join("|"));
            [first, second = "b", ...rest] = values(false);
            console.log(String(first), String(second), rest.join("|"));
            console.log(calls);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "union_array_destructuring"),
        "1 2 3|4\na b c|d\n1 2 3|4\na b c|d\n4\n"
    );
}

#[test]
fn nested_destructuring_reads_each_getter_result_once() {
    let source = r#"
        function main(): void {
            let reads = 0;
            const source = {
                get nested(): { x: number; y: number } {
                    reads++;
                    return { x: reads, y: reads };
                },
                get pair(): [number, number] {
                    reads++;
                    return [reads, reads];
                },
                get optional(): { x: number | undefined; y: number } {
                    reads++;
                    return { x: undefined, y: reads };
                },
            };
            const { nested: { x, y } } = source;
            console.log(x, y, reads);
            let assignedX = 0;
            let assignedY = 0;
            ({ nested: { x: assignedX, y: assignedY } } = source);
            console.log(assignedX, assignedY, reads);
            const { pair: [first, second] } = source;
            console.log(first, second, reads);
            let assignedFirst = 0;
            let assignedSecond = 0;
            ({ pair: [assignedFirst, assignedSecond] } = source);
            console.log(assignedFirst, assignedSecond, reads);
            const { optional: { x: defaulted = 42, y: sibling } } = source;
            console.log(defaulted, sibling, reads);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nested_destructuring_getter_once"),
        "1 1 1\n2 2 2\n3 3 3\n4 4 4\n42 5 5\n"
    );
}

#[test]
fn nested_destructuring_preserves_array_and_function_union_metadata() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        type Source = { nested: { items: Result[]; make: () => Result } };
        function main(): void {
            const source: Source = {
                nested: {
                    items: [{ kind: "number", value: 32 } as Result],
                    make: (): Result => ({ kind: "text", value: "returned" }),
                },
            };
            const { nested: { items, make } } = source;
            for (const item of items) {
                if (item.kind === "number") console.log(item.value + 10);
                else console.log(item.value + "!");
            }
            const made = make();
            if (made.kind === "number") console.log(made.value + 10);
            else console.log(made.value + "!");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nested_destructuring_union_metadata"),
        "42\nreturned!\n"
    );
}

#[test]
fn array_map_keeps_presence_at_each_visit() {
    let source = r#"
        function main(): void {
            const source = [1, 2, 3];
            const mapped = source.map((value: number, index: number) => {
                if (index === 1) { delete source[0]; }
                return value * 10;
            });
            console.log(0 in mapped, mapped[0]);
            const sparse = [1, 2, 3];
            delete sparse[1];
            const result = sparse.map((value: number, index: number) => {
                if (index === 2) { sparse[1] = 99; }
                return value * 10;
            });
            console.log(1 in result, 2 in result, result[2]);
        }
    "#;
    assert_eq!(compile_and_run(source, "array_map_visit_presence"), "true 10\nfalse true 30\n");
}

#[test]
fn array_filter_keeps_the_pre_predicate_value_and_presence() {
    let source = r#"
        function main(): void {
            const source = [1, 2];
            const filtered = source.filter((value: number, index: number) => {
                source[index] = 99;
                return value === 1;
            });
            console.log(filtered.length, filtered[0]);
            const values = [3, 4];
            const result = values.filter((value: number, index: number) => {
                delete values[index];
                return true;
            });
            console.log(result.length, 0 in result, result[0], 1 in result, result[1]);
        }
    "#;
    assert_eq!(compile_and_run(source, "array_filter_snapshot"), "1 1\n2 true 3 true 4\n");
}

#[test]
fn comparator_sort_uses_snapshot_and_writes_back_only_on_success() {
    let source = r#"
        function main(): void {
            const values = [2, 1];
            const result = values.sort((left: number, right: number): number => {
                values[0] = 99;
                return 0;
            });
            console.log(result === values, values.join(","));
            const failed = [2, 1];
            try {
                failed.sort((left: number, right: number): number => {
                    failed[0] = 99;
                    throw "stop";
                });
            } catch (error) {
                console.log(failed.join(","));
            }
            const extended = [2, 1];
            extended.sort((left: number, right: number): number => {
                extended.push(3);
                return 0;
            });
            console.log(extended.join(","));
        }
    "#;
    assert_eq!(compile_and_run(source, "sort_snapshot_mutation"),
        "true 2,1\n99,1\n2,1,3\n");
}

#[test]
fn computed_dictionary_assignment_default_evaluates_key_once() {
    let source = r#"
        let calls = 0;
        let defaults = 0;
        function key(): string { calls++; return calls === 1 ? "a" : "b"; }
        function fallback(): number { defaults++; return 99; }
        function main(): void {
            const present: Record<string, number> = { a: 7, b: 9 };
            let result = 0;
            ({ [key()]: result = fallback() } = present);
            console.log(result, calls, defaults);
            calls = 0;
            const missing: Record<string, number> = { b: 9 };
            ({ [key()]: result = fallback() } = missing);
            console.log(result, calls, defaults);
        }
    "#;
    assert_eq!(compile_and_run(source, "computed_dictionary_assignment_key_once"),
        "7 1 0\n99 1 1\n");
}

#[test]
fn dynamic_destructuring_defaults_check_read_value_and_preserve_null() {
    let source = r#"
        let defaults = 0;
        function fallback(): number { defaults++; return 42; }
        let keys = 0;
        function key(): string { keys++; return "present"; }
        function main(): void {
            const source: any = { present: undefined, nil: null, value: 7 };
            const { present = fallback(), missing = fallback(), nil = fallback(), value = fallback() } = source;
            console.log(present, missing, nil, value, defaults);
            let assigned: any = 0;
            ({ present: assigned = fallback() } = source);
            console.log(assigned, defaults);
            const { [key()]: computed = fallback() } = source;
            console.log(computed, keys, defaults);
            const array: any = [undefined, null, 7];
            const [first = fallback(), second = fallback(), third = fallback()] = array;
            console.log(first, second, third, defaults);
            let reads = 0;
            const access: any = { get value(): any { reads++; return undefined; } };
            const { value: accessed = fallback() } = access;
            console.log(accessed, reads, defaults);
        }
    "#;
    assert_eq!(compile_and_run(source, "dynamic_destructuring_value_defaults"),
        "42 42 null 7 2\n42 3\n42 1 4\n42 null 7 5\n42 1 6\n");
}

#[test]
fn missing_fixed_object_assignment_properties_use_defaults() {
    let source = r#"
        let defaults = 0;
        function fallback(): number { defaults++; return 99; }
        function main(): void {
            let assigned = 0;
            ({ assigned = fallback() } = {});
            console.log(assigned, defaults);
            const source = { present: 7 };
            const returned = ({ missing: assigned = fallback() } = source);
            console.log(assigned, defaults, returned.present);
            ({ present: assigned = fallback() } = source);
            console.log(assigned, defaults);
            let rest = { present: 0 };
            ({ missing: assigned = fallback(), ...rest } = source);
            console.log(assigned, defaults, rest.present);
        }
    "#;
    assert_eq!(compile_and_run(source, "missing_fixed_assignment_defaults"),
        "99 1\n99 2 7\n7 2\n99 3 7\n");
}

#[test]
fn dynamic_destructuring_validates_empty_patterns_and_snapshots_named_sources() {
    let source = r#"
        let reads = 0;
        let defaults = 0;
        function absent(): any { reads++; return null; }
        function fallback(): number { defaults++; return 42; }
        function main(): void {
            try { const {} = absent(); }
            catch (error) { console.log(error instanceof TypeError, reads, defaults); }
            try { const [] = absent(); }
            catch (error) { console.log(error instanceof TypeError, reads, defaults); }
            try { const { value = fallback() } = absent(); }
            catch (error) { console.log(error instanceof TypeError, reads, defaults); }
            try { ({} = absent()); }
            catch (error) { console.log(error instanceof TypeError, reads, defaults); }
            let original: any = { first: undefined, later: 7 };
            function replace(): number { original = { later: 99 }; return 42; }
            const { first = replace(), later } = original;
            console.log(first, later, original.later);
            let assigned: any = { first: undefined, later: 8 };
            let a = 0;
            let b = 0;
            function replaceAssigned(): number { assigned = { later: 100 }; return 43; }
            ({ first: a = replaceAssigned(), later: b } = assigned);
            console.log(a, b, assigned.later);
        }
    "#;
    assert_eq!(compile_and_run(source, "destructure_dynamic_source"),
        "true 1 0\ntrue 2 0\ntrue 3 0\ntrue 4 0\n42 7 99\n43 8 100\n");
}
