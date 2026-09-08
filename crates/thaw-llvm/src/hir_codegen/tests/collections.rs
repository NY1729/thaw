#[test]
fn compiles_native_array_join() {
    let source = r#"
        function separator(): string {
            console.log("separator-evaluated");
            return " | ";
        }
        function tupleValue(): [number, string, boolean, number[]] {
            console.log("receiver-evaluated");
            return [7, "x", false, [8, 9]];
        }
        async function delayed(): Promise<number[]> {
            await sleep(1);
            console.log("array-awaited");
            return [4, 5, 6];
        }
        function orderedValues(): number[] {
            console.log("join-receiver");
            return [1, 2];
        }
        async function delayedSeparator(): Promise<string> {
            console.log("join-separator");
            await sleep(1);
            return "|";
        }
        async function main(): Promise<void> {
            const numbers: number[] = [1, -0, 2.5];
            const words: string[] = ["a", "", "c"];
            const flags: boolean[] = [true, false];
            const objects: { value: number }[] = [{ value: 1 }, { value: 2 }];
            const empty: number[] = [];
            console.log(numbers.join());
            console.log(words.join("-"));
            console.log(flags.join(""));
            console.log(objects.join(" / "));
            console.log(tupleValue().join(separator()));
            console.log(empty.join("ignored"));
            console.log((await delayed()).join("+"));
            console.log(orderedValues().join(await delayedSeparator()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_join"),
        "1,0,2.5\na--c\ntruefalse\n[object Object] / [object Object]\nreceiver-evaluated\nseparator-evaluated\n7 | x | false | 8,9\n\narray-awaited\n4+5+6\njoin-receiver\njoin-separator\n1|2\n"
    );
}

#[test]
fn compiles_native_array_index_of_and_includes() {
    let source = r#"
        interface Item { value: number; }
        function values(): number[] {
            console.log("receiver");
            return [1, 2, 3];
        }
        function wrongNeedle(): string {
            console.log("needle");
            return "2";
        }
        function start(): boolean {
            console.log("start");
            return true;
        }
        async function delayedStart(): Promise<number> {
            await sleep(1);
            console.log("awaited-start");
            return -2;
        }
        function objectValues(first: Item, second: Item): Item[] {
            console.log("object-receiver");
            return [first, second];
        }
        async function delayedItem(item: Item): Promise<Item> {
            console.log("object-needle");
            await sleep(1);
            return item;
        }
        async function main(): Promise<void> {
            const numbers: number[] = [1, 2, 0 / 0, -0];
            const words: string[] = ["a", "b", "a"];
            const flags: boolean[] = [false, true, false];
            console.log(numbers.indexOf(2));
            console.log(numbers.indexOf(0 / 0));
            console.log(numbers.includes(0 / 0));
            console.log(numbers.includes(0));
            console.log(words.indexOf("a", 1));
            console.log(words.includes("a", -1));
            console.log(flags.indexOf(false, -2));
            console.log(numbers.includes(1, Number("Infinity")));
            console.log(values().includes(wrongNeedle(), start()));
            console.log(numbers.indexOf(0, await delayedStart()));
            const first: Item = { value: 1 };
            const second: Item = { value: 1 };
            console.log(objectValues(first, second).indexOf(await delayedItem(first)));
            console.log([first, second].includes({ value: 1 }));
            console.log([first, second, first].indexOf(first, 1));
            console.log(numbers.lastIndexOf(1));
            console.log(numbers.lastIndexOf(0));
            console.log(numbers.lastIndexOf(0 / 0));
            console.log(numbers.lastIndexOf(2, 0));
            console.log(numbers.lastIndexOf(1, 0 / 0));
            console.log(numbers.lastIndexOf(1, Number("-Infinity")));
            console.log(words.lastIndexOf("a"));
            console.log(words.lastIndexOf("a", 1));
            console.log(flags.lastIndexOf(false, -2));
            console.log(objectValues(first, second).lastIndexOf(await delayedItem(first)));
            console.log([first, second, first].lastIndexOf(first, 1));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_search"),
        "1\n-1\ntrue\ntrue\n2\ntrue\n2\nfalse\nreceiver\nneedle\nstart\nfalse\nawaited-start\n3\nobject-receiver\nobject-needle\n0\nfalse\n2\n0\n3\n-1\n-1\n0\n-1\n2\n0\n0\nobject-receiver\nobject-needle\n0\n0\n"
    );
}

#[test]
fn compiles_in_place_native_array_reverse() {
    let source = r#"
        async function delayed(): Promise<string[]> {
            await sleep(1);
            console.log("awaited-array");
            return ["a", "b", "c"];
        }
        async function main(): Promise<void> {
            const numbers: number[] = [1, 2, 3, 4];
            console.log(numbers.reverse().join("-"));
            console.log(numbers.join(","));
            const flags: boolean[] = [true, false, false];
            flags.reverse();
            console.log(flags.join("|"));
            const objects: { value: number }[] = [{ value: 1 }, { value: 2 }];
            objects.reverse();
            console.log(objects[0].value);
            console.log((await delayed()).reverse().join(""));
            const empty: number[] = [];
            console.log(empty.reverse().length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_reverse"),
        "4-3-2-1\n4,3,2,1\nfalse|false|true\n2\nawaited-array\ncba\n0\n"
    );
}

#[test]
fn compiles_in_place_native_array_fill() {
    let source = r#"
        interface Item { value: number; }
        async function end(): Promise<number> { await sleep(1); return -1; }
        async function main(): Promise<void> {
            const numbers: number[] = [1, 2, 3, 4];
            console.log(numbers.fill(9, 1, 3).join(","));
            const words: string[] = ["a", "b", "c"];
            words.fill("x", -2);
            console.log(words.join(""));
            const flags: boolean[] = [true, false, true];
            flags.fill(false);
            console.log(flags.join("-"));
            const item: Item = { value: 7 };
            const objects: Item[] = [{ value: 1 }, { value: 2 }];
            objects.fill(item, 0, await end());
            item.value = 8;
            console.log(objects[0].value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_fill"),
        "1,9,9,4\naxx\nfalse-false-false\n8\n"
    );
}

#[test]
fn compiles_native_array_concat() {
    let source = r#"
        interface Item { value: number; }
        function first(): number[] {
            console.log("receiver");
            return [1];
        }
        function second(): number[] {
            console.log("argument");
            return [2, 3];
        }
        async function delayed(): Promise<number[]> {
            console.log("awaited");
            await sleep(1);
            return [5, 6];
        }
        async function main(): Promise<void> {
            const numbers: number[] = first().concat(second(), 4, await delayed());
            console.log(numbers.join(","));
            const source: number[] = [7, 8];
            const copy: number[] = source.concat();
            copy[0] = 9;
            console.log(source.join(","));
            console.log(copy.join(","));
            console.log(["a"].concat(["b", "c"], "d").join(""));
            console.log([true].concat(false, [true]).join("-"));
            const item: Item = { value: 1 };
            const objects: Item[] = [item].concat([{ value: 2 }]);
            item.value = 9;
            console.log(objects[0].value);
            const empty: number[] = [];
            console.log(empty.concat([]).length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_concat"),
        "receiver\nargument\nawaited\n1,2,3,4,5,6\n7,8\n9,8\nabcd\ntrue-false-true\n9\n0\n"
    );
}

#[test]
fn compiles_non_mutating_native_array_slice() {
    let source = r#"
        function values(): number[] {
            console.log("receiver");
            return [1, 2, 3, 4];
        }
        function start(): string {
            console.log("start");
            return "1";
        }
        async function end(): Promise<number> {
            await sleep(1);
            console.log("awaited-end");
            return 3;
        }
        async function main(): Promise<void> {
            const numbers: number[] = [1, 2, 3, 4, 5];
            console.log(numbers.slice().join(","));
            console.log(numbers.slice(1, 4).join(","));
            console.log(numbers.slice(-3, -1).join(","));
            console.log(numbers.slice(4, 2).length);
            console.log(numbers.join(","));
            const words: string[] = ["a", "b", "c"];
            console.log(words.slice(1).join(""));
            const objects: { value: number }[] = [{ value: 1 }, { value: 2 }];
            const shallow: { value: number }[] = objects.slice(0, 1);
            shallow[0].value = 9;
            console.log(objects[0].value);
            console.log(values().slice(start(), await end()).join("-"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_slice"),
        "1,2,3,4,5\n2,3,4\n3,4\n0\n1,2,3,4,5\nbc\n9\nreceiver\nstart\nawaited-end\n2-3\n"
    );
}

#[test]
fn compiles_default_native_array_sorting() {
    let source = r#"
        interface Item { value: number; }
        interface Ranked { value: number; rank: number; }
        function descending(left: number, right: number): number {
            return right - left;
        }
        async function delayed(): Promise<string[]> {
            console.log("awaited");
            await sleep(1);
            return ["b", "a"];
        }
        async function main(): Promise<void> {
            const numbers: number[] = [10, 2, 1, 0 / 0, -0];
            console.log(numbers.sort().join(","));
            console.log(numbers.join(","));
            const words: string[] = ["ä", "z", "a", "😀"];
            const sortedWords: string[] = words.toSorted();
            console.log(sortedWords.join("|"));
            console.log(words.join("|"));
            const flags: boolean[] = [true, false, true, false];
            console.log(flags.toSorted().join("-"));
            const objects: Item[] = [{ value: 2 }, { value: 1 }];
            console.log(objects.toSorted()[0].value);
            console.log((await delayed()).toSorted().join(""));
            const empty: number[] = [];
            console.log(empty.sort().length);
            const descendingNumbers: number[] = [1, 3, 2];
            console.log(descendingNumbers.sort(descending).join(","));
            const source: number[] = [3, 1, 2];
            console.log(source.toSorted((left, right) => left - right).join(","));
            console.log(source.join(","));
            const direction: number = -1;
            console.log(source.toSorted((left, right) => (left - right) * direction).join(","));
            const ranked: Ranked[] = [
                { value: 2, rank: 1 },
                { value: 1, rank: 2 },
                { value: 2, rank: 3 }
            ];
            const ordered: Ranked[] = ranked.toSorted((left, right) => left.value - right.value);
            console.log(ordered[0].rank);
            console.log(ordered[1].rank);
            console.log(ordered[2].rank);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_default_sort"),
        "0,1,10,2,NaN\n0,1,10,2,NaN\na|z|ä|😀\nä|z|a|😀\nfalse-false-true-true\n2\nawaited\nab\n0\n3,2,1\n1,2,3\n3,1,2\n3,2,1\n2\n1\n3\n"
    );
}

#[test]
fn compiles_native_array_some_and_every() {
    let source = r#"
        interface Item { value: number; }
        function isSecond(value: number, index: number, array: number[]): boolean {
            console.log(value);
            return value === 2 && index === 1 && array.length === 3;
        }
        function belowFour(value: number, index: number, array: number[]): boolean {
            console.log(value);
            return value < 4 && index < array.length;
        }
        function values(): number[] {
            console.log("receiver");
            return [1, 2, 3];
        }
        function thisValue(): number {
            console.log("thisArg");
            return 1;
        }
        async function delayed(): Promise<string[]> {
            console.log("awaited");
            await sleep(1);
            return ["a", "", "c"];
        }
        async function main(): Promise<void> {
            console.log([1, 2, 3].some(isSecond));
            console.log([1, 2, 5, 3].every(belowFour));
            const threshold: number = 0;
            console.log([1, 2, 3].every(value => value > threshold));
            const empty: number[] = [];
            console.log(empty.some(() => true));
            console.log(empty.every(() => false));
            console.log([true, false].some(value => value === false));
            const item: Item = { value: 2 };
            console.log([item].every(value => value.value === 2));
            console.log(values().some(value => value === 3, thisValue()));
            console.log((await delayed()).every((value, index) => value.length === index));
            console.log([3, 5, 4].findIndex((value, index, array) => value === 4 && index === 2 && array.length === 3));
            console.log([1, 2].findIndex(value => value === 9));
            console.log(empty.findIndex(() => true));
            console.log([item].findIndex(value => value.value === 2));
            console.log((await delayed()).findIndex(value => value === ""));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_some_every"),
        "1\n2\ntrue\n1\n2\n5\nfalse\ntrue\nfalse\ntrue\ntrue\ntrue\nreceiver\nthisArg\ntrue\nawaited\nfalse\n2\n-1\n-1\n0\nawaited\n1\n"
    );
}

#[test]
fn compiles_native_array_find_and_find_last() {
    let source = r#"
        interface Item { value: number; }
        function receiver(): number[] {
            console.log("receiver");
            return [1, 2, 3, 2];
        }
        function thisValue(): string {
            console.log("thisArg");
            return "ignored";
        }
        async function delayed(): Promise<string[]> {
            console.log("awaited");
            await sleep(1);
            return ["a", "b", "a"];
        }
        function maybe(flag: boolean): number | undefined {
            if (flag) {
                return 42;
            }
            return undefined;
        }
        async function main(): Promise<void> {
            console.log([1, 2, 3].find(value => value === 2));
            console.log([1, 2, 3].find(value => value === 9));
            console.log(["a", "b"].find(value => value === "b"));
            console.log([true, false].find(value => value === false));
            const first: Item = { value: 1 };
            const second: Item = { value: 2 };
            console.log([first, second].find(item => item.value === 2));
            console.log(receiver().find((value, index, array) => value === 2 && index < array.length, thisValue()));
            console.log([1, 2, 3, 2].findLast((value, index) => {
                console.log(index);
                return value === 2;
            }));
            const empty: number[] = Array.of<number>();
            console.log(empty.find(() => true));
            console.log(empty.findLast(() => true));
            console.log((await delayed()).findLast(value => value === "a"));
            const missing: number | undefined = [1].find(value => value === 9);
            const present: number | undefined = [2].find(value => value === 2);
            console.log(missing === undefined);
            console.log(present !== undefined);
            console.log(undefined);
            console.log(maybe(true));
            console.log(maybe(false));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_find"),
        "2\nundefined\nb\nfalse\n{\"value\":2}\nreceiver\nthisArg\n2\n3\n2\nundefined\nundefined\nawaited\na\ntrue\ntrue\nundefined\n42\nundefined\n"
    );
}

#[test]
fn compiles_native_array_at() {
    let source = r#"
        interface Item { value: number; }
        function receiver(): number[] {
            console.log("receiver");
            return [1, 2, 3];
        }
        function index(): string {
            console.log("index");
            return "-1";
        }
        async function delayedReceiver(): Promise<string[]> {
            console.log("awaited receiver");
            await sleep(1);
            return ["a", "b"];
        }
        async function delayedIndex(): Promise<number> {
            console.log("awaited index");
            await sleep(1);
            return 0;
        }
        async function main(): Promise<void> {
            console.log([1, 2, 3].at(0));
            console.log([1, 2, 3].at(-1));
            console.log([1, 2, 3].at(1.9));
            console.log([1, 2, 3].at(0 / 0));
            console.log([1, 2, 3].at(3));
            console.log([1, 2, 3].at(-4));
            console.log([1, 2, 3].at(Infinity));
            console.log(["a", "b"].at(1));
            console.log([true, false].at(-1));
            const item: Item = { value: 1 };
            console.log([item].at(0));
            const empty: number[] = Array.of<number>();
            console.log(empty.at(0));
            console.log(receiver().at(index()));
            console.log((await delayedReceiver()).at(await delayedIndex()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_at"),
        "1\n3\n2\n1\nundefined\nundefined\nundefined\nb\nfalse\n{\"value\":1}\nundefined\nreceiver\nindex\n3\nawaited receiver\nawaited index\na\n"
    );
}

#[test]
fn compiles_native_array_for_each() {
    let source = r#"
        interface Item { value: number; }
        function visit(value: number, index: number, array: number[]): void {
            console.log(value + index + array.length);
        }
        function values(): number[] {
            console.log("receiver");
            return [1, 2];
        }
        function thisValue(): string {
            console.log("thisArg");
            return "ignored";
        }
        async function delayed(): Promise<string[]> {
            console.log("awaited");
            await sleep(1);
            return ["a", "b"];
        }
        async function main(): Promise<void> {
            [1, 2, 3].forEach(visit);
            let total: number = 0;
            [4, 5].forEach(value => { total = total + value; });
            console.log(total);
            const items: Item[] = [{ value: 6 }, { value: 7 }];
            items.forEach(item => { console.log(item.value); });
            const empty: number[] = [];
            empty.forEach(() => { console.log("wrong"); });
            values().forEach((value, index) => { console.log(value + index); }, thisValue());
            (await delayed()).forEach(value => { console.log(value); });
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_for_each"),
        "4\n6\n8\n9\n6\n7\nreceiver\nthisArg\n1\n3\nawaited\na\nb\n"
    );
}

#[test]
fn compiles_native_array_find_last_index() {
    let source = r#"
        interface Item { value: number; }
        function receiver(): number[] {
            console.log("receiver");
            return [1, 2, 3, 2];
        }
        function thisValue(): string {
            console.log("thisArg");
            return "ignored";
        }
        async function delayed(): Promise<string[]> {
            console.log("awaited");
            await sleep(1);
            return ["a", "b", "a"];
        }
        async function main(): Promise<void> {
            console.log([1, 2, 3, 2].findLastIndex((value, index, array) => {
                console.log(index);
                return value === 2 && array.length === 4;
            }));
            console.log([1, 2].findLastIndex(value => value === 9));
            const empty: number[] = [];
            console.log(empty.findLastIndex(() => true));
            const first: Item = { value: 1 };
            const second: Item = { value: 2 };
            console.log([first, second].findLastIndex(item => item.value < 3));
            console.log(receiver().findLastIndex(value => value === 2, thisValue()));
            console.log((await delayed()).findLastIndex(value => value === "a"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_find_last_index"),
        "3\n3\n-1\n-1\n1\nreceiver\nthisArg\n3\nawaited\n2\n"
    );
}

#[test]
fn compiles_native_array_reduce_and_reduce_right() {
    let source = r#"
        interface Box { value: number; }
        function sum(accumulator: number, value: number, index: number, array: number[]): number {
            console.log(index);
            return accumulator + value + array.length - 3;
        }
        function receiver(): number[] {
            console.log("receiver");
            return [1, 2, 3];
        }
        function initial(): number {
            console.log("initial");
            return 10;
        }
        async function delayed(): Promise<string[]> {
            console.log("awaited");
            await sleep(1);
            return ["a", "b", "c"];
        }
        async function main(): Promise<void> {
            console.log([1, 2, 3].reduce(sum));
            console.log(receiver().reduce((accumulator, value) => accumulator + value, initial()));
            console.log(["a", "b", "c"].reduceRight((accumulator, value) => accumulator + value, ""));
            console.log([1, 2].reduce((accumulator, value) => accumulator + String(value), ""));
            const empty: number[] = [];
            console.log(empty.reduce((accumulator, value) => accumulator + value, 7));
            const first: Box = { value: 1 };
            const second: Box = { value: 2 };
            console.log([first, second].reduce((accumulator, value) => value).value);
            console.log((await delayed()).reduceRight((accumulator, value) => accumulator + value));
            try {
                empty.reduce((accumulator, value) => accumulator + value);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_reduce"),
        "1\n2\n6\nreceiver\ninitial\n16\ncba\n12\n7\n2\nawaited\ncba\nReduce of empty array with no initial value\n"
    );
}

#[test]
fn compiles_native_array_map() {
    let source = r#"
        interface Item { value: number; }
        function project(value: number, index: number, array: number[]): number {
            return value * 2 + index + array.length;
        }
        function receiver(): number[] {
            console.log("receiver");
            return [1, 2];
        }
        function thisValue(): string {
            console.log("thisArg");
            return "ignored";
        }
        async function delayed(): Promise<string[]> {
            console.log("awaited");
            await sleep(1);
            return ["a", "b"];
        }
        async function main(): Promise<void> {
            console.log([1, 2, 3].map(project).join(","));
            console.log([1, 2].map(...[project]).join(","));
            const suffix: string = "!";
            console.log(["a", "b"].map((value, index) => value + String(index) + suffix).join("|"));
            console.log([true, false].map(value => value === false).join("-"));
            const items: Item[] = [{ value: 1 }, { value: 2 }];
            const mapped: Item[] = items.map(item => ({ value: item.value + 10 }));
            console.log(mapped[0].value + mapped[1].value);
            const nested: number[][] = [1, 2].map(value => [value]);
            console.log(nested[1][0]);
            const empty: number[] = [];
            console.log(empty.map(value => value + 1).length);
            console.log(receiver().map(value => value + 1, thisValue()).join(","));
            console.log((await delayed()).map(value => value + value).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_map"),
        "5,8,11\n4,7\na0!|b1!\nfalse-true\n23\n2\n0\nreceiver\nthisArg\n2,3\nawaited\naa,bb\n"
    );
}

#[test]
fn compiles_native_array_filter() {
    let source = r#"
        interface Item { value: number; }
        function even(value: number, index: number, array: number[]): boolean {
            console.log(index);
            return value % 2 === 0 && index < array.length;
        }
        function receiver(): number[] {
            console.log("receiver");
            return [1, 2, 3];
        }
        function thisValue(): string {
            console.log("thisArg");
            return "ignored";
        }
        async function delayed(): Promise<string[]> {
            console.log("awaited");
            await sleep(1);
            return ["", "a", "bb"];
        }
        async function main(): Promise<void> {
            console.log([1, 2, 3, 4].filter(even).join(","));
            const prefix: string = "a";
            console.log(["a", "b", "aa"].filter(value => value.startsWith(prefix)).join("|"));
            console.log([true, false, true].filter(value => value).join("-"));
            const first: Item = { value: 1 };
            const second: Item = { value: 2 };
            const items: Item[] = [first, second];
            const selected: Item[] = items.filter(item => item.value === 2);
            selected[0].value = 9;
            console.log(items[1].value);
            const empty: number[] = [];
            console.log(empty.filter(() => true).length);
            console.log(receiver().filter(value => value > 1, thisValue()).join(","));
            console.log((await delayed()).filter(value => value.length > 0).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_filter"),
        "0\n1\n2\n3\n2,4\na|aa\ntrue-true\n9\n0\nreceiver\nthisArg\n2,3\nawaited\na,bb\n"
    );
}

#[test]
fn compiles_native_array_with() {
    let source = r#"
        interface Item { value: number; }
        function receiver(): number[] {
            console.log("receiver");
            return [1, 2, 3];
        }
        function index(): number {
            console.log("index");
            return 1;
        }
        function value(): number {
            console.log("value");
            return 9;
        }
        async function delayedReceiver(): Promise<string[]> {
            console.log("awaited receiver");
            await sleep(1);
            return ["a", "b"];
        }
        async function delayedIndex(): Promise<number> {
            console.log("awaited index");
            await sleep(1);
            return -1;
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited value");
            await sleep(1);
            return "z";
        }
        async function main(): Promise<void> {
            const source: number[] = [1, 2, 3];
            console.log(source.with(1, 9).join(","));
            console.log(source.join(","));
            console.log(source.with(-1.9, 8).join(","));
            console.log(source.with(0 / 0, 7).join(","));
            const first: Item = { value: 1 };
            const second: Item = { value: 2 };
            const replacement: Item = { value: 3 };
            const items: Item[] = [first, second];
            const copied: Item[] = items.with(1, replacement);
            copied[0].value = 5;
            console.log(items[0].value + copied[1].value);
            console.log(receiver().with(index(), value()).join(","));
            console.log((await delayedReceiver()).with(await delayedIndex(), await delayedValue()).join(","));
            try {
                source.with(3, 0);
            } catch (error) {
                console.log(error);
            }
            const empty: number[] = [];
            try {
                empty.with(0, 1);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_with"),
        "1,9,3\n1,2,3\n1,2,8\n7,2,3\n8\nreceiver\nindex\nvalue\n1,9,3\nawaited receiver\nawaited index\nawaited value\na,z\nInvalid index for Array.prototype.with\nInvalid index for Array.prototype.with\n"
    );
}

#[test]
fn compiles_native_array_to_spliced() {
    let source = r#"
        interface Item { value: number; }
        function receiver(): number[] {
            console.log("receiver");
            return [1, 2, 3];
        }
        function start(): number {
            console.log("start");
            return 1;
        }
        function deletion(): number {
            console.log("delete");
            return 1;
        }
        function item(): number {
            console.log("item");
            return 9;
        }
        async function delayedReceiver(): Promise<string[]> {
            console.log("awaited receiver");
            await sleep(1);
            return ["a", "b", "c"];
        }
        async function delayedStart(): Promise<number> {
            console.log("awaited start");
            await sleep(1);
            return -1;
        }
        async function delayedItem(): Promise<string> {
            console.log("awaited item");
            await sleep(1);
            return "z";
        }
        async function main(): Promise<void> {
            const source: number[] = [1, 2, 3, 4];
            console.log(source.toSpliced().join(","));
            console.log(source.toSpliced(2).join(","));
            console.log(source.toSpliced(-2, 1, 8, 9).join(","));
            console.log(source.toSpliced(0 / 0, 0 / 0, 7).join(","));
            console.log(source.toSpliced(1.9, 1.9, 6).join(","));
            console.log(source.toSpliced(99, 5, 8).join(","));
            console.log(source.toSpliced(-99, -2, 8).join(","));
            console.log(source.toSpliced(...[1, 2, 8, 9]).join(","));
            console.log(source.join(","));
            const first: Item = { value: 1 };
            const second: Item = { value: 2 };
            const replacement: Item = { value: 3 };
            const objects: Item[] = [first, second];
            const copied: Item[] = objects.toSpliced(1, 1, replacement);
            copied[0].value = 5;
            console.log(objects[0].value + copied[1].value);
            console.log(receiver().toSpliced(start(), deletion(), item()).join(","));
            console.log((await delayedReceiver()).toSpliced(await delayedStart(), 1, await delayedItem()).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_to_spliced"),
        "1,2,3,4\n1,2\n1,2,8,9,4\n7,1,2,3,4\n1,6,3,4\n1,2,3,4,8\n8,1,2,3,4\n1,8,9,4\n1,2,3,4\n8\nreceiver\nstart\ndelete\nitem\n1,9,3\nawaited receiver\nawaited start\nawaited item\na,b,z\n"
    );
}

#[test]
fn compiles_native_array_flat_map() {
    let source = r#"
        interface Item { value: number; }
        function expand(value: number, index: number, array: number[]): number[] {
            console.log(index);
            return [value, array.length];
        }
        function receiver(): number[] {
            console.log("receiver");
            return [1, 2];
        }
        function thisValue(): string {
            console.log("thisArg");
            return "ignored";
        }
        async function delayed(): Promise<string[]> {
            console.log("awaited");
            await sleep(1);
            return ["a", "b"];
        }
        async function main(): Promise<void> {
            console.log([1, 2].flatMap(expand).join(","));
            const suffix: string = "!";
            console.log(["a", "b"].flatMap(value => [value, value + suffix]).join("|"));
            console.log([true, false].flatMap(value => [value, value]).join("-"));
            const first: Item = { value: 1 };
            const second: Item = { value: 2 };
            const items: Item[] = [first, second];
            const copied: Item[] = items.flatMap(item => [item]);
            copied[0].value = 9;
            console.log(items[0].value);
            const nested: number[][] = [1, 2].flatMap(value => [[value], [value + 10]]);
            console.log(nested[2][0]);
            const empty: number[] = [];
            console.log(empty.flatMap(value => [value]).length);
            console.log(receiver().flatMap(value => [value, value + 1], thisValue()).join(","));
            console.log((await delayed()).flatMap(value => [value + value]).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_flat_map"),
        "0\n1\n1,2,2,2\na|a!|b|b!\ntrue-true-false-false\n9\n2\n0\nreceiver\nthisArg\n1,2,2,3\nawaited\naa,bb\n"
    );
}

#[test]
fn compiles_native_array_flat() {
    let source = r#"
        interface Item { value: number; }
        async function delayed(): Promise<string[][]> {
            console.log("awaited");
            await sleep(1);
            const first: string[] = ["a"];
            const empty: string[] = ["x"].slice(0, 0);
            const last: string[] = ["b", "c"];
            const result: string[][] = [first, empty, last];
            return result;
        }
        async function main(): Promise<void> {
            const nested: number[][] = [[1, 2], [], [3]];
            console.log(nested.flat().join(","));
            const deep: number[][][] = [[[1], [2]], [[3]]];
            const once: number[][] = deep.flat();
            console.log(once.length);
            console.log(once[1][0]);
            console.log(deep.flat(2).join(","));
            console.log(deep.flat(1.9).length);
            const unchangedDepth: number[][][] = deep.flat(-1);
            console.log(unchangedDepth.length);
            const source: number[] = [1, 2];
            const copied: number[] = source.flat();
            copied[0] = 9;
            console.log(source.join(","));
            const item: Item = { value: 1 };
            const objects: Item[][] = [[item]];
            const flattened: Item[] = objects.flat();
            flattened[0].value = 7;
            console.log(item.value);
            const empty: number[][] = [[0]].slice(0, 0);
            console.log(empty.flat().length);
            console.log((await delayed()).flat().join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_flat"),
        "1,2,3\n3\n2\n1,2,3\n3\n2\n1,2\n7\n0\nawaited\na,b,c\n"
    );
}

#[test]
fn compiles_tuple_spreads_for_array_positional_methods() {
    let source = r#"
        function main(): void {
            console.log([1, 2, 3].join(...["-"]));
            console.log([1, 2, 3].at(...[-1]));
            console.log([1, 2, 3].with(...[1, 9]).join(","));
            const nested: number[][] = [[1], [2, 3]];
            console.log(nested.flat(...[1]).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_positional_tuple_spreads"),
        "1-2-3\n3\n1,9,3\n1,2,3\n"
    );
}

#[test]
fn compiles_tuple_spreads_for_array_callbacks() {
    let source = r#"
        function double(value: number): number { return value * 2; }
        function even(value: number): boolean { return value % 2 === 0; }
        function expand(value: number): number[] { return [value, value]; }
        function emit(value: number): void { console.log(value); }
        function compare(left: number, right: number): number { return left - right; }
        function sum(total: number, value: number): number { return total + value; }
        function main(): void {
            console.log([1, 2].map(...[double]).join(","));
            console.log([1, 2, 3].filter(...[even]).join(","));
            console.log([1, 2].some(...[even]));
            console.log([1, 2].flatMap(...[expand]).join(","));
            [3, 4].forEach(...[emit]);
            console.log([3, 1, 2].toSorted(...[compare]).join(","));
            console.log([1, 2, 3].reduce(...[sum, 0]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_callback_tuple_spreads"),
        "2,4\n2\ntrue\n1,1,2,2\n3\n4\n1,2,3\n6\n"
    );
}

#[test]
fn compiles_tuple_spreads_for_array_range_methods() {
    let source = r#"
        function main(): void {
            console.log([1, 2, 3, 4].slice(...[1, 3]).join(","));
            const copied: number[] = [1, 2, 3, 4];
            console.log(copied.copyWithin(...[0, 2, 4]).join(","));
            const filled: number[] = [1, 2, 3, 4];
            console.log(filled.fill(...[9, 1, 3]).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_range_tuple_spreads"),
        "2,3\n3,4,3,4\n1,9,9,4\n"
    );
}

#[test]
fn compiles_native_array_of() {
    let source = r#"
        interface Item { value: number; }
        function first(): number {
            console.log("first");
            return 1;
        }
        function middle(): number[] {
            console.log("middle");
            return [2, 3];
        }
        function last(): number {
            console.log("last");
            return 4;
        }
        async function delayed(): Promise<number[]> {
            console.log("awaited");
            await sleep(1);
            return [6, 7];
        }
        async function main(): Promise<void> {
            console.log(Array.of(1, 2, 3).join(","));
            console.log(Array.of("a", "b").join("|"));
            console.log(Array.of(true, false).join("-"));
            const item: Item = { value: 5 };
            const objects: Item[] = Array.of(item);
            objects[0].value = 9;
            console.log(item.value);
            const nested: number[][] = Array.of([1], [2, 3]);
            console.log(nested[1][1]);
            const empty: string[] = Array.of<string>();
            console.log(empty.length);
            console.log(Array.of(first(), ...middle(), last()).join(","));
            console.log(Array.of(5, ...(await delayed()), 8).join(","));
            // Spread sources get the same string/Map/Set snapshot
            // conversion array literals and `Array.from` already do.
            console.log(Array.of(...new Set<number>([1, 2, 2, 3])).join(","));
            console.log(Array.of(..."ab").join("|"));
            const entries = Array.of(
                ...new Map<string, number>([["x", 1], ["y", 2]]),
            );
            for (const [k, v] of entries) {
                console.log(k, v);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_of"),
        "1,2,3\na|b\ntrue-false\n9\n3\n0\nfirst\nmiddle\nlast\n1,2,3,4\nawaited\n5,6,7,8\n1,2,3\na|b\nx 1\ny 2\n"
    );
}

#[test]
fn compiles_native_array_from_native_arrays() {
    let source = r#"
        interface Item { value: number; }
        function project(value: number, index: number): number {
            return value * 2 + index;
        }
        function receiver(): number[] {
            console.log("receiver");
            return [1, 2];
        }
        function thisValue(): string {
            console.log("thisArg");
            return "ignored";
        }
        async function delayed(): Promise<string[]> {
            console.log("awaited");
            await sleep(1);
            return ["a", "b"];
        }
        function text(): string {
            console.log("text");
            return "A😀é";
        }
        async function delayedText(): Promise<string> {
            console.log("awaited text");
            await sleep(1);
            return "😀a";
        }
        async function main(): Promise<void> {
            const source: number[] = [1, 2, 3];
            const copied: number[] = Array.from(source);
            copied[0] = 9;
            console.log(source.join(","));
            console.log(Array.from(source, project).join(","));
            console.log(Array.from<number, string>(source, value => String(value) + "!").join("|"));
            const item: Item = { value: 1 };
            const objects: Item[] = Array.from(Array.of(item));
            objects[0].value = 7;
            console.log(item.value);
            const emptySource: boolean[] = Array.of<boolean>();
            console.log(Array.from<boolean>(emptySource).length);
            console.log(Array.from(receiver(), (value, index) => value + index, thisValue()).join(","));
            console.log(Array.from(await delayed(), value => value + value).join(","));
            console.log(Array.from(text()).join("|"));
            console.log(Array.from<string, string>("ab", (value, index) => value + String(index), thisValue()).join(","));
            console.log(Array.from(await delayedText()).length);
            console.log(Array.from(...[source]).join(","));
            console.log(Array.from(...[source, project]).join(","));
            console.log(Array.from(...[source, project, thisValue()]).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_from"),
        "1,2,3\n2,5,8\n1!|2!|3!\n7\n0\nreceiver\nthisArg\n1,3\nawaited\naa,bb\ntext\nA|😀|é\nthisArg\na0,b1\nawaited text\n2\n1,2,3\n2,5,8\nthisArg\n2,5,8\n"
    );
}

#[test]
fn compiles_native_array_from_a_map_or_set() {
    // Reuses the exact same `__thaw_map_snapshot_entries`/
    // `__thaw_map_snapshot_keys` conversion array-literal spreads
    // (`[...map]`/`[...set]`) already use, so `Array.from` matches each
    // container's own default iterator shape: `[key, value]` pairs for a
    // `Map`, elements (deduplicated, insertion order) for a `Set`.
    let source = r#"
        async function main(): Promise<void> {
            const s = new Set<number>([1, 2, 2, 3]);
            console.log(Array.from(s).join(","));
            console.log(Array.from(s, value => value * 10).join(","));
            const m = new Map<string, number>([["a", 1], ["b", 2]]);
            for (const [k, v] of Array.from(m)) {
                console.log(k, v);
            }
            const emptySet = new Set<number>();
            console.log(Array.from(emptySet).length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_from_map_or_set"),
        "1,2,3\n10,20,30\na 1\nb 2\n0\n"
    );
}

#[test]
fn compiles_native_array_from_an_array_like_length_object() {
    // `Array.from({length})` -- as opposed to the real-array/string source
    // the test above covers. A plain `{ length }` object has no indexed
    // properties in this compiler's fixed-layout object model, so every
    // per-index value the spec would read from the source is `undefined`,
    // matching real JavaScript for a source object with no other own
    // properties; `mapfn` typically ignores it and uses only the index.
    let source = r#"
        function thisValue(): string {
            console.log("thisArg");
            return "ignored";
        }
        function lengthOf(count: number): number {
            console.log("length");
            return count;
        }
        function nextIndex(value: undefined, index: number): number {
            return index + 1;
        }
        async function main(): Promise<void> {
            const empty = Array.from({ length: 0 });
            console.log(empty.length);
            const bare = Array.from({ length: 3 });
            console.log(bare.length);
            const doubled: number[] = Array.from({ length: 4 }, (_, i) => i * 2);
            console.log(doubled.join(","));
            const words: string[] = Array.from(
                { length: lengthOf(3) },
                (_, i) => `w${i}`,
                thisValue(),
            );
            console.log(words.join(","));
            // Spread arguments are lowered generically (no contextual typing
            // for an inline callback's own parameters), so this leg needs a
            // fully-typed named function rather than `(_, i) => ...` -- the
            // same requirement the array/string overload's own spread tests
            // above already live with.
            console.log(Array.from(...[{ length: 2 }], nextIndex).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_from_length"),
        "0\n3\n0,2,4,6\nlength\nthisArg\nw0,w1,w2\n1,2\n"
    );
}

#[test]
fn compiles_map_group_by_a_native_array() {
    // Built entirely from the same generic primitives `.get()`/`.set()`/
    // `.push()` already lower to, plus `__thaw_map_new`/`ArrayAlloc` --
    // no new runtime or codegen, so this mostly exercises composition:
    // repeated keys accumulate via `.push()`'s in-place handle mutation
    // rather than re-inserting a fresh bucket on every match.
    let source = r#"
        function lengthKey(word: string, index: number): number {
            return word.length;
        }
        async function main(): Promise<void> {
            const items: number[] = [1, 2, 3, 4, 5, 6];
            const groups: Map<string, number[]> = Map.groupBy(items, (value, index) => {
                console.log("key", value, index);
                return value % 2 === 0 ? "even" : "odd";
            });
            const evens = groups.get("even");
            if (evens !== undefined) {
                console.log(evens.join(","));
            }
            const odds = groups.get("odd");
            if (odds !== undefined) {
                console.log(odds.join(","));
            }
            console.log(groups.size);
            console.log(groups.has("missing"));

            const words: string[] = ["a", "bb", "cc", "ddd"];
            const bySpread: Map<number, string[]> = Map.groupBy(...[words, lengthKey]);
            const two = bySpread.get(2);
            if (two !== undefined) {
                console.log(two.join(","));
            }

            const empty: number[] = [];
            const emptyGroups: Map<string, number[]> = Map.groupBy(empty, (v, i) => "x");
            console.log(emptyGroups.size);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "map_group_by"),
        concat!(
            "key 1 0\n", "key 2 1\n", "key 3 2\n", "key 4 3\n", "key 5 4\n", "key 6 5\n",
            "2,4,6\n", "1,3,5\n", "2\n", "false\n", "bb,cc\n", "0\n",
        )
    );
}

#[test]
fn compiles_set_union_intersection_and_difference() {
    // Each builds a fresh Set from `__thaw_map_snapshot_keys` snapshots of
    // its operand(s), the same conversion `[...set]`/`Array.from(set)`
    // already use -- neither operand is mutated.
    let source = r#"
        async function main(): Promise<void> {
            const a = new Set<number>([1, 2, 3]);
            const b = new Set<number>([2, 3, 4]);
            console.log(Array.from(a.union(b)).join(","));
            console.log(Array.from(a.intersection(b)).join(","));
            console.log(Array.from(a.difference(b)).join(","));
            console.log(a.size, b.size);

            const empty = new Set<number>();
            console.log(Array.from(a.union(empty)).join(","));
            console.log(Array.from(a.intersection(empty)).join(","));
            console.log(Array.from(a.difference(empty)).join(","));
            console.log(Array.from(empty.union(a)).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "set_union_intersection_difference"),
        concat!(
            "1,2,3,4\n", "2,3\n", "1\n", "3 3\n",
            "1,2,3\n", "\n", "1,2,3\n", "1,2,3\n",
        )
    );
}

#[test]
fn compiles_set_symmetric_difference_and_relational_predicates() {
    // `symmetricDifference` shares `union`'s two-pass shape but keeps an
    // element only when the *other* side lacks it. `isSubsetOf`/
    // `isSupersetOf`/`isDisjointFrom` share the same snapshot/`.has()`
    // shape but return a boolean via a short-circuiting scan instead of
    // building a new Set; `isSupersetOf` is implemented as the same scan
    // with receiver/argument swapped rather than a separate algorithm.
    let source = r#"
        async function main(): Promise<void> {
            const a = new Set<number>([1, 2, 3]);
            const b = new Set<number>([2, 3, 4]);
            console.log(Array.from(a.symmetricDifference(b)).join(","));
            console.log(a.isSubsetOf(b));
            console.log(a.isSubsetOf(new Set<number>([1, 2, 3, 4])));
            console.log(a.isSupersetOf(new Set<number>([1, 2])));
            console.log(a.isSupersetOf(b));
            console.log(a.isDisjointFrom(b));
            console.log(a.isDisjointFrom(new Set<number>([9, 10])));
            console.log(a.size, b.size);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "set_symmetric_difference_and_predicates"),
        "1,4\nfalse\ntrue\ntrue\nfalse\nfalse\ntrue\n3 3\n"
    );
}

