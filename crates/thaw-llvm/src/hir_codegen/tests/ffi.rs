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
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_of"),
        "1,2,3\na|b\ntrue-false\n9\n3\n0\nfirst\nmiddle\nlast\n1,2,3,4\nawaited\n5,6,7,8\n"
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
fn compiles_variadic_string_concat() {
    let source = r#"
        function receiver(): string {
            console.log("receiver");
            return "value=";
        }
        function argument(): number {
            console.log("argument");
            return 42;
        }
        async function delayed(): Promise<boolean> {
            await sleep(1);
            console.log("awaited-concat");
            return true;
        }
        async function main(): Promise<void> {
            const values: number[] = [1, 2];
            console.log(receiver().concat(argument(), ";values=", values, ";object=", { x: 1 }));
            console.log("empty".concat());
            console.log("flag=".concat(await delayed()));
            console.log(receiver().concat(argument(), await delayed()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_concat_method"),
        "receiver\nargument\nvalue=42;values=1,2;object=[object Object]\nempty\nawaited-concat\nflag=true\nreceiver\nargument\nawaited-concat\nvalue=42true\n"
    );
}

#[test]
fn compiles_native_string_repeat() {
    let source = r#"
        function text(): string {
            console.log("receiver");
            return "ab";
        }
        function count(): number {
            console.log("count");
            return 2;
        }
        async function delayedText(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "😀";
        }
        async function delayedCount(): Promise<number> {
            console.log("awaited count");
            await sleep(1);
            return 2;
        }
        async function main(): Promise<void> {
            console.log("ab".repeat(3));
            console.log("😀".repeat(2));
            console.log("xy".repeat(2.9));
            console.log("x".repeat(0 / 0).length);
            console.log("z".repeat("2"));
            console.log(text().repeat(count()));
            console.log((await delayedText()).repeat(await delayedCount()));
            try {
                "x".repeat(-1);
            } catch (error) {
                console.log(error);
            }
            try {
                "x".repeat(Infinity);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_repeat"),
        "ababab\n😀😀\nxyxy\n0\nzz\nreceiver\ncount\nabab\nawaited receiver\nawaited count\n😀😀\nInvalid count value for String.prototype.repeat\nInvalid count value for String.prototype.repeat\n"
    );
}

#[test]
fn compiles_native_string_pad_start_and_pad_end() {
    let source = r#"
        function text(): string {
            console.log("receiver");
            return "5";
        }
        function length(): number {
            console.log("length");
            return 3;
        }
        function pad(): string {
            console.log("pad");
            return "0";
        }
        async function delayedText(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "😀";
        }
        async function main(): Promise<void> {
            console.log("5".padStart(3, "0"));
            console.log("5".padEnd(3, "0"));
            console.log("abc".padStart(2, "0"));
            console.log("5".padStart(3));
            console.log("1".padStart(5, "ab"));
            console.log("x".padStart(5, ""));
            console.log("5".padStart(3, 0));
            console.log("😀".padStart(3, "x"));
            console.log(text().padStart(length(), pad()));
            console.log((await delayedText()).padStart(3, "x"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_pad_start_and_end"),
        "005\n500\nabc\n  5\nabab1\nx\n005\nx😀\nreceiver\nlength\npad\n005\nawaited receiver\nx😀\n"
    );
}

#[test]
fn compiles_native_number_to_fixed() {
    let source = r#"
        function value(): number {
            console.log("receiver");
            return 1.5;
        }
        function digits(): number {
            console.log("digits");
            return 2;
        }
        async function delayedValue(): Promise<number> {
            console.log("awaited receiver");
            await sleep(1);
            return 3.14159;
        }
        async function main(): Promise<void> {
            console.log((1.5).toFixed(2));
            console.log((3.7).toFixed());
            console.log((5).toFixed());
            console.log((-1.005).toFixed(2));
            console.log((1e21).toFixed(2));
            console.log((0 / 0).toFixed(2));
            console.log(value().toFixed(digits()));
            console.log((await delayedValue()).toFixed(3));
            try {
                (1).toFixed(-1);
            } catch (error) {
                console.log(error);
            }
            try {
                (1).toFixed(101);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "number_to_fixed"),
        "1.50\n4\n5\n-1.00\n1e+21\nNaN\nreceiver\ndigits\n1.50\nawaited receiver\n3.142\ntoFixed() digits argument must be between 0 and 100\ntoFixed() digits argument must be between 0 and 100\n"
    );
}

#[test]
fn compiles_native_string_code_point_at() {
    let source = r#"
        function text(): string {
            console.log("receiver");
            return "a";
        }
        function index(): number {
            console.log("index");
            return 0;
        }
        async function delayedText(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "😀";
        }
        async function main(): Promise<void> {
            console.log("a".codePointAt(0));
            console.log("a".codePointAt());
            console.log("😀".codePointAt(0));
            console.log("😀".codePointAt(1));
            console.log("a".codePointAt(5));
            console.log("a".codePointAt(-1));
            console.log(text().codePointAt(index()));
            console.log((await delayedText()).codePointAt(0));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_code_point_at"),
        "97\n97\n128512\n56832\nundefined\nundefined\nreceiver\nindex\n97\nawaited receiver\n128512\n"
    );
}

#[test]
fn compiles_encode_uri_component() {
    let source = r#"
        function value(): string {
            console.log("argument");
            return "a=1&b=2";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited argument");
            await sleep(1);
            return "😀";
        }
        async function main(): Promise<void> {
            console.log(encodeURIComponent("a b"));
            console.log(encodeURIComponent("a=1&b=2"));
            console.log(encodeURIComponent("abc-_.!~*'()123"));
            console.log(encodeURIComponent("café"));
            console.log(encodeURIComponent(value()));
            console.log(encodeURIComponent(await delayedValue()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "encode_uri_component"),
        "a%20b\na%3D1%26b%3D2\nabc-_.!~*'()123\ncaf%C3%A9\nargument\na%3D1%26b%3D2\nawaited argument\n%F0%9F%98%80\n"
    );
}

#[test]
fn compiles_decode_uri_component() {
    let source = r#"
        function value(): string {
            console.log("argument");
            return "a%3D1%26b%3D2";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited argument");
            await sleep(1);
            return "%F0%9F%98%80";
        }
        async function main(): Promise<void> {
            console.log(decodeURIComponent("a%20b"));
            console.log(decodeURIComponent("a%3D1%26b%3D2"));
            console.log(decodeURIComponent("abc-_.!~*'()123"));
            console.log(decodeURIComponent("caf%C3%A9"));
            console.log(decodeURIComponent(value()));
            console.log(decodeURIComponent(await delayedValue()));
            try {
                decodeURIComponent("%");
            } catch (error) {
                console.log(error);
            }
            try {
                decodeURIComponent("%zz");
            } catch (error) {
                console.log(error);
            }
            try {
                decodeURIComponent("%C3");
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "decode_uri_component"),
        "a b\na=1&b=2\nabc-_.!~*'()123\ncafé\nargument\na=1&b=2\nawaited argument\n😀\nURI malformed\nURI malformed\nURI malformed\n"
    );
}

#[test]
fn compiles_native_number_to_precision() {
    let source = r#"
        function value(): number {
            console.log("receiver");
            return 123.456;
        }
        function digits(): number {
            console.log("digits");
            return 4;
        }
        async function delayedValue(): Promise<number> {
            console.log("awaited receiver");
            await sleep(1);
            return 123456;
        }
        async function main(): Promise<void> {
            console.log((123.456).toPrecision(4));
            console.log((0.00001234).toPrecision(2));
            console.log((123456).toPrecision(2));
            console.log((0).toPrecision(3));
            console.log((-123.456).toPrecision(4));
            console.log((42).toPrecision());
            console.log((0 / 0).toPrecision(3));
            console.log(value().toPrecision(digits()));
            console.log((await delayedValue()).toPrecision(2));
            try {
                (1).toPrecision(0);
            } catch (error) {
                console.log(error);
            }
            try {
                (1).toPrecision(101);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "number_to_precision"),
        "123.5\n0.000012\n1.2e+5\n0.00\n-123.5\n42\nNaN\nreceiver\ndigits\n123.5\nawaited receiver\n1.2e+5\ntoPrecision() argument must be between 1 and 100\ntoPrecision() argument must be between 1 and 100\n"
    );
}

#[test]
fn compiles_encode_uri() {
    let source = r#"
        function value(): string {
            console.log("argument");
            return "http://a.com/a b";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited argument");
            await sleep(1);
            return "😀";
        }
        async function main(): Promise<void> {
            console.log(encodeURI("http://a.com/a b?x=1&y=2#frag"));
            console.log(encodeURI(";/?:@&=+$,#"));
            console.log(encodeURI("café"));
            console.log(encodeURI(value()));
            console.log(encodeURI(await delayedValue()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "encode_uri"),
        "http://a.com/a%20b?x=1&y=2#frag\n;/?:@&=+$,#\ncaf%C3%A9\nargument\nhttp://a.com/a%20b\nawaited argument\n%F0%9F%98%80\n"
    );
}

#[test]
fn compiles_decode_uri() {
    let source = r#"
        function value(): string {
            console.log("argument");
            return "http://a.com/a%20b";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited argument");
            await sleep(1);
            return "%F0%9F%98%80";
        }
        async function main(): Promise<void> {
            console.log(decodeURI("http://a.com/a%20b?x=1&y=2#frag"));
            console.log(decodeURI("%3B%2F%3F%3A%40%26%3D%2B%24%2C%23"));
            console.log(decodeURI("a%20%2Fb"));
            console.log(decodeURI("caf%C3%A9"));
            console.log(decodeURI(value()));
            console.log(decodeURI(await delayedValue()));
            try {
                decodeURI("%");
            } catch (error) {
                console.log(error);
            }
            try {
                decodeURI("%zz");
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "decode_uri"),
        "http://a.com/a b?x=1&y=2#frag\n%3B%2F%3F%3A%40%26%3D%2B%24%2C%23\na %2Fb\ncafé\nargument\nhttp://a.com/a b\nawaited argument\n😀\nURI malformed\nURI malformed\n"
    );
}

#[test]
fn compiles_string_from_char_code() {
    let source = r#"
        function first(): number {
            console.log("first");
            return 72;
        }
        function second(): number {
            console.log("second");
            return 101;
        }
        async function delayedCode(): Promise<number> {
            console.log("awaited code");
            await sleep(1);
            return 65;
        }
        async function main(): Promise<void> {
            console.log(String.fromCharCode());
            console.log(String.fromCharCode(65));
            console.log(String.fromCharCode(72, 101, 108, 108, 111));
            console.log(String.fromCharCode(65.9));
            console.log(String.fromCharCode(65 + 65536));
            console.log(String.fromCharCode(first(), second()));
            console.log(String.fromCharCode(await delayedCode()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_from_char_code"),
        "\nA\nHello\nA\nA\nfirst\nsecond\nHe\nawaited code\nA\n"
    );
}

#[test]
fn compiles_string_from_code_point() {
    let source = r#"
        function first(): number {
            console.log("first");
            return 65;
        }
        function second(): number {
            console.log("second");
            return 128512;
        }
        async function delayedPoint(): Promise<number> {
            console.log("awaited point");
            await sleep(1);
            return 128512;
        }
        async function main(): Promise<void> {
            console.log(String.fromCodePoint());
            console.log(String.fromCodePoint(65));
            console.log(String.fromCodePoint(72, 101, 108, 108, 111));
            console.log(String.fromCodePoint(128512));
            console.log(String.fromCodePoint(65, 128512));
            console.log(String.fromCodePoint(first(), second()));
            console.log(String.fromCodePoint(await delayedPoint()));
            try {
                String.fromCodePoint(-1);
            } catch (error) {
                console.log(error);
            }
            try {
                String.fromCodePoint(1114112);
            } catch (error) {
                console.log(error);
            }
            try {
                String.fromCodePoint(65.5);
            } catch (error) {
                console.log(error);
            }
            try {
                String.fromCodePoint(55296);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_from_code_point"),
        "\nA\nHello\n😀\nA😀\nfirst\nsecond\nA😀\nawaited point\n😀\nInvalid code point\nInvalid code point\nInvalid code point\nInvalid code point\n"
    );
}

#[test]
fn compiles_string_locale_compare() {
    let source = r#"
        function receiver(): string {
            console.log("receiver");
            return "a";
        }
        function other(): string {
            console.log("other");
            return "b";
        }
        async function delayedOther(): Promise<string> {
            console.log("awaited other");
            await sleep(1);
            return "a";
        }
        async function main(): Promise<void> {
            console.log("a".localeCompare("b"));
            console.log("b".localeCompare("a"));
            console.log("a".localeCompare("a"));
            console.log(receiver().localeCompare(other()));
            console.log((await delayedOther()).localeCompare("a"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_locale_compare"),
        "-1\n1\n0\nreceiver\nother\n-1\nawaited other\n0\n"
    );
}

#[test]
fn compiles_string_normalize() {
    let source = r#"
        function value(): string {
            console.log("receiver");
            return "é";
        }
        function form(): string {
            console.log("form");
            return "NFC";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "é";
        }
        async function main(): Promise<void> {
            console.log("é".normalize("NFC"));
            console.log("é".normalize());
            console.log("é".normalize("NFD"));
            console.log("ﬁ".normalize("NFKD"));
            console.log(value().normalize(form()));
            console.log((await delayedValue()).normalize("NFC"));
            try {
                "abc".normalize("bogus");
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_normalize"),
        "é\né\ne\u{301}\nfi\nreceiver\nform\né\nawaited receiver\né\nThe normalization form should be one of NFC, NFD, NFKC, NFKD.\n"
    );
}

#[test]
fn compiles_tagged_template_literals() {
    let source = r#"
        function upper(strings: string[], value: number): string {
            return strings[0] + value + strings[1].toUpperCase();
        }
        function greeting(strings: string[]): string {
            return strings[0].toUpperCase();
        }
        function values(strings: string[], a: number, b: number): number {
            return strings.length + a + b;
        }
        function main(): void {
            console.log(upper`count: ${5} done`);
            console.log(greeting`hello`);
            console.log(values`${1}mid${2}`);
            console.log(String.raw`a\nb${1 + 1}c`);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tagged_template_literals"),
        "count: 5 DONE\nHELLO\n6\na\\nb2c\n"
    );
}

#[test]
fn compiles_tagged_template_evaluation_order_and_async() {
    let source = r#"
        const combine = (strings: string[], a: number, b: number): string => {
            return strings[0] + a + strings[1] + b + strings[2];
        };
        function first(): number {
            console.log("first");
            return 1;
        }
        function second(): number {
            console.log("second");
            return 2;
        }
        async function delayed(): Promise<number> {
            console.log("awaited value");
            await sleep(1);
            return 3;
        }
        async function main(): Promise<void> {
            console.log(combine`[${first()}|${second()}]`);
            console.log(combine`[${await delayed()}|${first()}]`);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tagged_template_order_and_async"),
        "first\nsecond\n[1|2]\nawaited value\nfirst\n[3|1]\n"
    );
}

#[test]
fn compiles_native_string_split() {
    let source = r#"
        function printAll(parts: string[]): void {
            console.log(parts.length);
            for (const part of parts) {
                console.log(part);
            }
        }
        function value(): string {
            console.log("receiver");
            return "a,b,c";
        }
        function separator(): string {
            console.log("separator");
            return ",";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "x::y::z";
        }
        async function main(): Promise<void> {
            printAll("a,b,c".split(","));
            printAll("abc".split(""));
            printAll("a,b,c".split(",", 2));
            printAll("hello".split());
            printAll("a::b::c".split("::"));
            printAll("abc".split("x"));
            printAll(value().split(separator()));
            printAll((await delayedValue()).split("::"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_string_split"),
        "3\na\nb\nc\n3\na\nb\nc\n2\na\nb\n1\nhello\n3\na\nb\nc\n1\nabc\nreceiver\nseparator\n3\na\nb\nc\nawaited receiver\n3\nx\ny\nz\n"
    );
}

#[test]
fn compiles_native_string_replace() {
    let source = r#"
        function value(): string {
            console.log("receiver");
            return "abc abc";
        }
        function search(): string {
            console.log("search");
            return "a";
        }
        function replacement(): string {
            console.log("replacement");
            return "X";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "abc abc";
        }
        async function main(): Promise<void> {
            console.log("abc abc".replace("a", "X"));
            console.log("abc abc".replaceAll("a", "X"));
            console.log("abc".replace("x", "y"));
            console.log("abc".replaceAll("x", "y"));
            console.log("abc".replace("", "X"));
            console.log("abc".replaceAll("", "X"));
            console.log(value().replace(search(), replacement()));
            console.log((await delayedValue()).replaceAll("a", "X"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_string_replace"),
        "Xbc abc\nXbc Xbc\nabc\nabc\nXabc\nXaXbXcX\nreceiver\nsearch\nreplacement\nXbc abc\nawaited receiver\nXbc Xbc\n"
    );
}

#[test]
fn compiles_regex_test() {
    let source = r#"
        function pattern(): RegExp {
            console.log("pattern");
            return /abc/;
        }
        function value(): string {
            console.log("value");
            return "xabcx";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited value");
            await sleep(1);
            return "xabcx";
        }
        async function main(): Promise<void> {
            console.log(/abc/.test("xabcx"));
            console.log(/abc/.test("xyz"));
            console.log(new RegExp("abc").test("xabcx"));
            console.log(/ABC/i.test("xabcx"));
            console.log(/^abc$/.test("abc"));
            console.log(/^abc$/.test("xabc"));
            console.log(pattern().test(value()));
            console.log(/abc/.test(await delayedValue()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_test"),
        "true\nfalse\ntrue\ntrue\ntrue\nfalse\npattern\nvalue\ntrue\nawaited value\ntrue\n"
    );
}

#[test]
fn compiles_string_search() {
    let source = r#"
        function value(): string {
            console.log("value");
            return "xabcx";
        }
        function pattern(): RegExp {
            console.log("pattern");
            return /abc/i;
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited value");
            await sleep(1);
            return "xabcx";
        }
        async function main(): Promise<void> {
            console.log("xabcx".search(/abc/));
            console.log("xyz".search(/abc/));
            console.log("xABCx".search(/abc/i));
            console.log(value().search(pattern()));
            console.log((await delayedValue()).search(/abc/));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_search"),
        "1\n-1\n1\nvalue\npattern\n1\nawaited value\n1\n"
    );
}

#[test]
fn compiles_date_getters_and_iso_string() {
    let source = r#"
        function fixed(): Date {
            console.log("fixed");
            return new Date(1704067200500);
        }
        async function main(): Promise<void> {
            const d: Date = new Date(1704067200500);
            console.log(d.getTime());
            console.log(d.valueOf());
            console.log(d.getFullYear());
            console.log(d.getMonth());
            console.log(d.getDate());
            console.log(d.getDay());
            console.log(d.getHours());
            console.log(d.getMinutes());
            console.log(d.getSeconds());
            console.log(d.getMilliseconds());
            console.log(d.getUTCFullYear());
            console.log(d.toISOString());
            console.log(fixed().toISOString());
            d.setTime(0);
            console.log(d.toISOString());
            console.log(Date.now() > 0);
            try {
                new Date(NaN).toISOString();
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "date_getters"),
        "1.70407e+12\n1.70407e+12\n2024\n0\n1\n1\n0\n0\n0\n500\n2024\n2024-01-01T00:00:00.500Z\nfixed\n2024-01-01T00:00:00.500Z\n1970-01-01T00:00:00.000Z\ntrue\nInvalid time value\n"
    );
}

#[test]
fn compiles_date_setters() {
    let source = r#"
        async function main(): Promise<void> {
            const d: Date = new Date(1704067200500);
            console.log(d.setDate(15));
            console.log(d.toISOString());
            const e: Date = new Date(1704067200500);
            e.setMonth(11);
            console.log(e.toISOString());
            // Month index 12 (one past December) rolls into next January.
            e.setMonth(12);
            console.log(e.toISOString());
            const f: Date = new Date(1704067200500);
            f.setFullYear(2000, 5);
            console.log(f.toISOString());
            const g: Date = new Date(1704067200500);
            // Day 0 of the month moves to the last day of the previous one.
            g.setDate(0);
            console.log(g.toISOString());
            const h: Date = new Date(1704067200500);
            h.setHours(25);
            console.log(h.toISOString());
            const j: Date = new Date(1704067200500);
            j.setSeconds(90);
            console.log(j.toISOString());
            const k: Date = new Date(1704067200500);
            k.setMilliseconds(NaN);
            console.log(k.getTime());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "date_setters"),
        "1.70528e+12\n2024-01-15T00:00:00.500Z\n2024-12-01T00:00:00.500Z\n2025-01-01T00:00:00.500Z\n2000-06-01T00:00:00.500Z\n2023-12-31T00:00:00.500Z\n2024-01-02T01:00:00.500Z\n2024-01-01T00:01:30.500Z\nnan\n"
    );
}

#[test]
fn compiles_date_utc_and_parse() {
    let source = r#"
        function isoText(): string {
            console.log("isoText");
            return "2024-01-01T00:00:00.500Z";
        }
        async function main(): Promise<void> {
            console.log(Date.UTC(2024, 0, 1, 0, 0, 0, 500));
            console.log(Date.UTC(2024));
            console.log(Date.UTC(70));
            console.log(Date.parse("2024-01-01T00:00:00.500Z"));
            console.log(Date.parse("2024-01-01"));
            console.log(Date.parse("not a date"));
            const d: Date = new Date("2024-01-01T00:00:00.500Z");
            console.log(d.toISOString());
            const e: Date = new Date(isoText());
            console.log(e.getTime());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "date_utc_parse"),
        "1.70407e+12\n1.70407e+12\n0\n1.70407e+12\n1.70407e+12\nnan\n2024-01-01T00:00:00.500Z\nisoText\n1.70407e+12\n"
    );
}

#[test]
fn compiles_date_string_formatting() {
    let source = r#"
        async function main(): Promise<void> {
            const d: Date = new Date(1704067200500);
            console.log(d.toDateString());
            console.log(d.toTimeString());
            console.log(d.toString());
            console.log(d.toUTCString());
            const invalid: Date = new Date(NaN);
            console.log(invalid.toDateString());
            console.log(invalid.toTimeString());
            console.log(invalid.toString());
            console.log(invalid.toUTCString());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "date_string_formatting"),
        "Mon Jan 01 2024\n00:00:00 GMT+0000 (Coordinated Universal Time)\nMon Jan 01 2024 00:00:00 GMT+0000 (Coordinated Universal Time)\nMon, 01 Jan 2024 00:00:00 GMT\nInvalid Date\nInvalid Date\nInvalid Date\nInvalid Date\n"
    );
}

#[test]
fn compiles_regex_exec() {
    let source = r#"
        function printMatch(result: string[] | undefined): void {
            if (result !== undefined) {
                for (const part of result) {
                    console.log(part);
                }
            } else {
                console.log("no match");
            }
        }
        function pattern(): RegExp {
            console.log("pattern");
            return /(\d+)-(\d+)/;
        }
        function value(): string {
            console.log("value");
            return "12-34";
        }
        async function main(): Promise<void> {
            printMatch(/(\d+)-(\d+)/.exec("12-34"));
            printMatch(/(\d+)-(\d+)/.exec("abc"));
            printMatch(pattern().exec(value()));
            printMatch(/(\d+)-(\d+)/g.exec("12-34"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_exec"),
        "12-34\n12\n34\nno match\npattern\nvalue\n12-34\n12\n34\n12-34\n12\n34\n"
    );
}

#[test]
fn compiles_string_match() {
    let source = r#"
        function printMatch(result: string[] | undefined): void {
            if (result !== undefined) {
                for (const part of result) {
                    console.log(part);
                }
            } else {
                console.log("no match");
            }
        }
        function value(): string {
            console.log("value");
            return "a1b2c3";
        }
        function pattern(): RegExp {
            console.log("pattern");
            return /\d/g;
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited value");
            await sleep(1);
            return "abc123def";
        }
        async function main(): Promise<void> {
            printMatch("abc123def".match(/\d+/));
            printMatch("a1b2c3".match(/\d/g));
            printMatch("abc".match(/\d+/));
            printMatch(value().match(pattern()));
            printMatch((await delayedValue()).match(/\d+/));
            printMatch("12-34".match(/(\d+)-(\d+)/));
            printMatch("a=1".match(/(a)=(\d)|(b)=(\d)/));
            printMatch("12-34".match(/(\d+)-(\d+)/g));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_match"),
        "123\n1\n2\n3\nno match\nvalue\npattern\n1\n2\n3\nawaited value\n123\n12-34\n12\n34\na=1\na\n1\n\n\n12-34\n"
    );
}

#[test]
fn compiles_regex_split_and_replace() {
    let source = r#"
        function printAll(parts: string[]): void {
            for (const part of parts) {
                console.log(part);
            }
        }
        async function main(): Promise<void> {
            printAll("a1b2c3".split(/\d/));
            printAll("a b  c".split(/\s+/));
            console.log("a1b2c3".replace(/\d/, "X"));
            console.log("a1b2c3".replace(/\d/g, "X"));
            console.log("a1b2c3".replaceAll(/\d/g, "X"));
            try {
                "a1b2c3".replaceAll(/\d/, "X");
            } catch (error) {
                console.log(error);
            }
            try {
                "abc".split(/(?=x)/);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_split_and_replace"),
        "a\nb\nc\n\na\nb\nc\naXb2c3\naXbXcX\naXbXcX\nreplaceAll must be called with a global RegExp\ninvalid regular expression\n"
    );
}

#[test]
fn compiles_well_formed_native_strings() {
    let source = r#"
        function text(): string {
            console.log("receiver");
            return "A😀é";
        }
        async function delayed(): Promise<string> {
            console.log("awaited");
            await sleep(1);
            return "later😀";
        }
        async function main(): Promise<void> {
            console.log("plain".isWellFormed());
            console.log("😀".toWellFormed());
            console.log(text().isWellFormed());
            console.log(text().toWellFormed());
            console.log((await delayed()).isWellFormed());
            console.log((await delayed()).toWellFormed());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "well_formed_strings"),
        "true\n😀\nreceiver\ntrue\nreceiver\nA😀é\nawaited\ntrue\nawaited\nlater😀\n"
    );
}

#[test]
fn compiles_number_predicates_and_aggregate_numeric_conversion() {
    let source = r#"
        function text(): string {
            console.log("strict-predicate-evaluated");
            return "bad";
        }
        async function delayed(value: string): Promise<string> {
            await sleep(1);
            console.log("awaited-predicate");
            return value;
        }
        async function main(): Promise<void> {
            console.log(Number.isNaN(0 / 0));
            console.log(Number.isNaN(text()));
            console.log(Number.isFinite(42));
            console.log(Number.isFinite(Number("Infinity")));
            console.log(isNaN("bad"));
            console.log(isNaN("12"));
            console.log(isFinite("12"));
            console.log(isFinite("Infinity"));
            const empty: number[] = [];
            const one: number[] = [7];
            const many: number[] = [1, 2];
            console.log(Number(empty));
            console.log(Number(one));
            console.log(isNaN(many));
            console.log(isNaN({ value: 1 }));
            console.log(isNaN(await delayed("9")));
            console.log(Number(...["12"]));
            console.log(String(...[true]));
            console.log(Boolean(...[0]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "number_predicates"),
        "true\nstrict-predicate-evaluated\nfalse\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse\n0\n7\ntrue\ntrue\nawaited-predicate\nfalse\n12\ntrue\nfalse\n"
    );
}

/// docs/design/bridge.md's first vertical slice: an ambient `declare
/// function` call actually links against and calls a real native
/// implementation -- here a tiny hand-written C function, standing in
/// for what would eventually be a thaw-registry-fetched library.
#[test]
fn compiles_ambient_declaration_and_links_a_real_native_function() {
    let source = r#"
        declare function native_add(a: number, b: number): number;

        function main(): void {
            console.log(native_add(2, 3));
        }
    "#;

    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    assert_eq!(
        program.extern_functions.len(),
        1,
        "sanity: this is really an FFI call"
    );

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_ambient");
    compiler.compile_program(&program).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi_ambient-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");

    compiler.write_object_file(&obj_path).unwrap();

    std::fs::write(
        &native_c_path,
        "double native_add(double a, double b) { return a + b; }\n",
    )
    .unwrap();
    let cc_status = Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .expect("failed to invoke `cc` to build the native stand-in library");
    assert!(cc_status.success(), "compiling native.c failed");

    let arena_lib = build_staticlib("thaw-arena");
    let link_status = Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    let output = Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    assert!(output.status.success(), "binary exited non-zero");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "5\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ffi_variadic_number_rest_calls_real_c_varargs() {
    let source = r#"
        declare function native_sum(count: number, ...values: number[]): number;
        declare function native_true_count(count: number, ...values: boolean[]): number;
        declare function native_total_length(count: number, ...values: string[]): number;
        declare function native_handle(): JsValue;
        declare function native_handle_sum(count: number, ...values: JsValue[]): number;
        declare function native_array_total(count: number, ...values: number[][]): number;
        declare function native_string_array_total(count: number, ...values: string[][]): number;
        declare function native_bool_array_total(count: number, ...values: boolean[][]): number;
        declare function native_handle_array_total(count: number, ...values: JsValue[][]): number;
        declare function native_object_total(
            count: number,
            ...values: {
                value: number;
                meta: { enabled: boolean; label: string };
                samples: number[];
            }[]
        ): number;
        declare function native_optional_total(
            count: number, ...values: (number | undefined)[]
        ): number;
        declare function native_nullable_total(
            count: number, ...values: (number | null)[]
        ): number;
        declare function native_nullish_total(
            count: number, ...values: (number | null | undefined)[]
        ): number;
        declare function native_optional_object_total(
            count: number, ...values: ({ value: number } | undefined)[]
        ): number;
        declare function native_sum_i32(count: number, ...values: number[]): number;
        declare function native_sum_i64(count: number, ...values: number[]): number;
        declare function native_sum_u32(count: number, ...values: number[]): number;
        declare function native_sum_u64(count: number, ...values: number[]): number;

        function main(): void {
            console.log(native_sum(0));
            console.log(native_sum(3, 2, 3, 5));
            console.log(native_true_count(4, true, false, true, true));
            console.log(native_total_length(3, "thaw", "ffi", "ok"));
            console.log(native_handle_sum(2, native_handle(), native_handle()));
            console.log(native_array_total(2, [1, 2], [3, 4, 5]));
            console.log(native_string_array_total(2, ["a", "bc"], ["def"]));
            console.log(native_bool_array_total(2, [true, false, true], [false, true]));
            console.log(native_handle_array_total(
                2,
                [native_handle()],
                [native_handle(), native_handle()]
            ));
            console.log(native_object_total(
                2,
                { value: 2, meta: { enabled: true, label: "abc" }, samples: [1, 2] },
                { value: 4, meta: { enabled: false, label: "x" }, samples: [3, 4, 5] }
            ));
            console.log(native_optional_total(2, 2, undefined));
            console.log(native_nullable_total(2, 3, null));
            console.log(native_nullish_total(3, 4, null, undefined));
            console.log(native_optional_object_total(2, { value: 5 }, undefined));
            console.log(native_sum_i32(2, 0 - 2, 5));
            console.log(native_sum_i64(2, 0 - 4, 10));
            console.log(native_sum_u32(2, 20, 22));
            console.log(native_sum_u64(2, 40, 2));
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    assert!(program
        .extern_functions
        .iter()
        .any(|signature| signature.variadic == Some(HirType::F64)));
    assert!(program
        .extern_functions
        .iter()
        .any(|signature| signature.variadic == Some(HirType::Bool)));
    assert!(program
        .extern_functions
        .iter()
        .any(|signature| signature.variadic == Some(HirType::Str)));
    assert!(program
        .extern_functions
        .iter()
        .any(|signature| signature.variadic == Some(HirType::JsValue)));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Array(Box::new(HirType::F64)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Array(Box::new(HirType::Str)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Array(Box::new(HirType::Bool)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Array(Box::new(HirType::JsValue)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        matches!(&signature.variadic, Some(HirType::Object(fields)) if fields.len() == 3)
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Optional(Box::new(HirType::F64)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Nullable(Box::new(HirType::F64)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Nullish(Box::new(HirType::F64)))
    }));
    for (symbol, abi) in [
        ("native_sum_i32", thaw_hir::FfiVariadicAbi::I32),
        ("native_sum_i64", thaw_hir::FfiVariadicAbi::I64),
        ("native_sum_u32", thaw_hir::FfiVariadicAbi::U32),
        ("native_sum_u64", thaw_hir::FfiVariadicAbi::U64),
    ] {
        thaw_hir::set_ffi_variadic_abi(&mut program, symbol, abi).unwrap();
    }
    assert!(thaw_hir::set_ffi_variadic_abi(
        &mut program,
        "native_true_count",
        thaw_hir::FfiVariadicAbi::I32,
    )
    .unwrap_err()
    .contains("requires a number[] rest parameter"));

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_variadic");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-variadic-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdarg.h>\n#include <stdint.h>\n#include <string.h>\n\
         double native_sum(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, double);\n\
           va_end(args); return sum;\n\
         }\n\
         double native_true_count(double raw_count, ...) {\n\
           int count = (int)raw_count; int found = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) found += va_arg(args, int) != 0;\n\
           va_end(args); return found;\n\
         }\n\
         double native_total_length(double raw_count, ...) {\n\
           int count = (int)raw_count; size_t length = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) length += strlen(va_arg(args, const char *));\n\
           va_end(args); return (double)length;\n\
         }\n\
         uint64_t native_handle(void) { return 20; }\n\
         double native_handle_sum(double raw_count, ...) {\n\
           int count = (int)raw_count; uint64_t sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, uint64_t);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_array_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             const double *data = va_arg(args, const double *);\n\
             long long length = va_arg(args, long long);\n\
             for (long long j = 0; j < length; ++j) sum += data[j];\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_string_array_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             const char *const *data = va_arg(args, const char *const *);\n\
             long long length = va_arg(args, long long);\n\
             for (long long j = 0; j < length; ++j) sum += strlen(data[j]);\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_bool_array_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             const int *data = va_arg(args, const int *);\n\
             long long length = va_arg(args, long long);\n\
             for (long long j = 0; j < length; ++j) sum += data[j] != 0;\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_handle_array_total(double raw_count, ...) {\n\
           int count = (int)raw_count; uint64_t sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             const uint64_t *data = va_arg(args, const uint64_t *);\n\
             long long length = va_arg(args, long long);\n\
             for (long long j = 0; j < length; ++j) sum += data[j];\n\
           }\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_object_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             sum += va_arg(args, double);\n\
             sum += va_arg(args, int) != 0;\n\
             sum += (double)strlen(va_arg(args, const char *));\n\
             const double *data = va_arg(args, const double *);\n\
             long long length = va_arg(args, long long);\n\
             for (long long j = 0; j < length; ++j) sum += data[j];\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_optional_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             sum += va_arg(args, int); sum += va_arg(args, double);\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_nullable_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             sum += va_arg(args, int); sum += va_arg(args, double);\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_nullish_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             sum += va_arg(args, int); sum += va_arg(args, double);\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_optional_object_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             sum += va_arg(args, int); sum += va_arg(args, double);\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_sum_i32(double raw_count, ...) {\n\
           int count = (int)raw_count; int sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, int);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_sum_i64(double raw_count, ...) {\n\
           int count = (int)raw_count; long long sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, long long);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_sum_u32(double raw_count, ...) {\n\
           int count = (int)raw_count; unsigned int sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, unsigned int);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_sum_u64(double raw_count, ...) {\n\
           int count = (int)raw_count; unsigned long long sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, unsigned long long);\n\
           va_end(args); return (double)sum;\n\
         }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "0\n10\n3\n9\n40\n15\n6\n3\n60\n26\n3\n4\n7\n6\n3\n6\n42\n42\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_thaw_result_abi_propagates_native_errors_into_try_catch() {
    let source = r#"
        declare function native_read(value: number): number;

        function main(): void {
            console.log(native_read(2));
            try {
                console.log(native_read(0 - 1));
                console.log("unreachable");
            } catch (error) {
                console.log(error);
            } finally {
                console.log("cleanup");
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_read",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_result_abi");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-result-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "typedef struct { double value; const char *error; } ThawF64Result;\n\
         ThawF64Result native_read(double value) {\n\
           if (value < 0) return (ThawF64Result){0, \"native read failed\"};\n\
           return (ThawF64Result){value * 10, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "20\nnative read failed\ncleanup\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_void_calls_support_direct_and_thaw_result_error_abis() {
    let source = r#"
        declare function native_mark(value: number): void;
        declare function mark_count(): number;
        declare function native_check(value: number): void;
        declare function error_destroy_count(): number;

        function main(): void {
            native_mark(2);
            console.log(mark_count());
            native_check(1);
            console.log("ok");
            try {
                native_check(0 - 1);
                console.log("unreachable");
            } catch (error) {
                console.log(error);
                console.log(error_destroy_count());
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_check",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();
    thaw_hir::set_ffi_ownership(
        &mut program,
        "native_check",
        thaw_hir::FfiOwnership::Borrowed,
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_error".into(),
        },
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_void_result");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-void-result-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdlib.h>\n#include <string.h>\n\
         typedef struct { char *error; } ThawVoidResult;\n\
         static int marks; static int error_destroys;\n\
         static char *copy(const char *s) { size_t n = strlen(s) + 1; char *p = malloc(n); memcpy(p, s, n); return p; }\n\
         void native_mark(double value) { marks += (int)value; }\n\
         double mark_count(void) { return marks; }\n\
         ThawVoidResult native_check(double value) {\n\
           if (value < 0) return (ThawVoidResult){copy(\"void check failed\")};\n\
           return (ThawVoidResult){0};\n\
         }\n\
         void destroy_error(char *p) { ++error_destroys; free(p); }\n\
         double error_destroy_count(void) { return error_destroys; }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "2\nok\nvoid check failed\n1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_owned_result_strings_are_copied_and_destroyed_once() {
    let source = r#"
        declare function native_read(value: number): string;
        declare function return_destroy_count(): number;
        declare function error_destroy_count(): number;

        function main(): void {
            console.log(native_read(1));
            console.log(return_destroy_count());
            try {
                console.log(native_read(0 - 1));
            } catch (error) {
                console.log(error);
                console.log(error_destroy_count());
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_read",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();
    thaw_hir::set_ffi_ownership(
        &mut program,
        "native_read",
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_return".into(),
        },
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_error".into(),
        },
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_owned_result");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-owned-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdlib.h>\n#include <string.h>\n\
         typedef struct { char *value; char *error; } ThawStringResult;\n\
         static int return_destroys; static int error_destroys;\n\
         static char *copy(const char *s) { size_t n = strlen(s) + 1; char *p = malloc(n); memcpy(p, s, n); return p; }\n\
         ThawStringResult native_read(double value) {\n\
           if (value < 0) return (ThawStringResult){0, copy(\"owned error\")};\n\
           return (ThawStringResult){copy(\"owned value\"), 0};\n\
         }\n\
         void destroy_return(char *p) { ++return_destroys; free(p); }\n\
         void destroy_error(char *p) { ++error_destroys; free(p); }\n\
         double return_destroy_count(void) { return return_destroys; }\n\
         double error_destroy_count(void) { return error_destroys; }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "owned value\n1\nowned error\n1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_array_and_object_returns_from_portable_c_structs() {
    let source = r#"
        declare function native_values(): number[];
        declare function native_point(): { x: number; y: number };
        declare function native_vector(): { x: number; y: number; z: number };
        declare function array_destroy_count(): number;

        function main(): void {
            const values: number[] = native_values();
            const point: { x: number; y: number } = native_point();
            const vector: { x: number; y: number; z: number } = native_vector();
            console.log(values[0] + values[1] + values[2]);
            console.log(point.x + point.y);
            console.log(vector.x + vector.y + vector.z);
            console.log(array_destroy_count());
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_ownership(
        &mut program,
        "native_values",
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_values".into(),
        },
        thaw_hir::FfiOwnership::Borrowed,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_values",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_point",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_vector",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_aggregate_returns");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-aggregate-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n#include <stdlib.h>\n\
         typedef struct { double *data; int64_t len; } ThawF64Array;\n\
         typedef struct { double x; double y; } Point;\n\
         typedef struct { double x; double y; double z; } Vector;\n\
         static int destroys;\n\
         ThawF64Array native_values(void) { double *p = malloc(3 * sizeof(double)); p[0] = 2; p[1] = 3; p[2] = 5; return (ThawF64Array){p, 3}; }\n\
         void destroy_values(void *p) { ++destroys; free(p); }\n\
         double array_destroy_count(void) { return destroys; }\n\
         Point native_point(void) { return (Point){7, 11}; }\n\
         Vector native_vector(void) { return (Vector){13, 17, 19}; }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "10\n18\n49\n1\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_object_returns_from_packed_c_structs() {
    let source = r#"
        declare function native_record(): { active: boolean; value: number };
        declare function native_checked_record(value: number): { active: boolean; value: number };

        function main(): void {
            const record: { active: boolean; value: number } = native_record();
            console.log(record.active);
            console.log(record.value);
            const checked: { active: boolean; value: number } = native_checked_record(7);
            console.log(checked.value);
            try {
                native_checked_record(0 - 1);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_record",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Packed,
    )
    .unwrap();
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_checked_record",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_checked_record",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Packed,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_packed_return");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-packed-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "typedef struct __attribute__((packed)) { _Bool active; double value; } PackedRecord;\n\
         typedef struct { PackedRecord value; const char *error; } PackedRecordResult;\n\
         PackedRecord native_record(void) { return (PackedRecord){1, 42}; }\n\
         PackedRecordResult native_checked_record(double value) {\n\
           if (value < 0) return (PackedRecordResult){{0, 0}, \"packed check failed\"};\n\
           return (PackedRecordResult){{1, value * 2}, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "true\n42\n14\npacked check failed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_uses_explicit_c_aggregate_offsets_and_alignment() {
    let source = r#"
        declare function native_aligned_record(): { active: boolean; value: number };
        declare function native_checked_aligned_record(value: number): { active: boolean; value: number };
        declare function native_nested_aligned_record(): { meta: { active: boolean; value: number }; total: number };
        declare function native_bit_record(): { active: boolean; ready: boolean; count: number; delta: number; value: number };
        declare function native_register_record(): { active: boolean; value: number };
        declare function native_checked_register_record(value: number): { active: boolean; value: number };

        function main(): void {
            const record: { active: boolean; value: number } = native_aligned_record();
            console.log(record.active);
            console.log(record.value);
            const checked: { active: boolean; value: number } = native_checked_aligned_record(7);
            console.log(checked.value);
            try {
                native_checked_aligned_record(0 - 1);
            } catch (error) {
                console.log(error);
            }
            const nested = native_nested_aligned_record();
            console.log(nested.meta.active);
            console.log(nested.meta.value + nested.total);
            const bits = native_bit_record();
            console.log(bits.active);
            console.log(bits.ready);
            console.log(bits.count);
            console.log(bits.delta);
            console.log(bits.value);
            const register = native_register_record();
            console.log(register.active);
            console.log(register.value);
            const checkedRegister = native_checked_register_record(9);
            console.log(checkedRegister.value);
            try {
                native_checked_register_record(0 - 1);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_aligned_record",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let mut invalid_program = program.clone();
    assert!(thaw_hir::set_ffi_aggregate_layout(
        &mut invalid_program,
        "native_aligned_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 0],
            field_layouts: vec![None, None],
            field_bitfields: vec![None, None],
            register_classes: vec![],
            size: 16,
            alignment: 8,
            indirect: true,
        },
    )
    .unwrap_err()
    .contains("outside or overlaps"));
    thaw_hir::set_ffi_aggregate_layout(
        &mut program,
        "native_aligned_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 16],
            field_layouts: vec![None, None],
            field_bitfields: vec![None, None],
            register_classes: vec![],
            size: 32,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap();
    for symbol in ["native_register_record", "native_checked_register_record"] {
        if symbol == "native_checked_register_record" {
            thaw_hir::set_ffi_error_abi(
                &mut program,
                symbol,
                thaw_hir::FfiErrorAbi::ThawResult,
            )
            .unwrap();
        }
        thaw_hir::set_ffi_string_abi(
            &mut program,
            symbol,
            if symbol == "native_register_record" {
                vec![]
            } else {
                vec![thaw_hir::FfiStringAbi::NullTerminated]
            },
            thaw_hir::FfiStringAbi::NullTerminated,
            thaw_hir::FfiCallingConvention::C,
            thaw_hir::FfiAggregateAbi::Portable,
        )
        .unwrap();
        thaw_hir::set_ffi_aggregate_layout(
            &mut program,
            symbol,
            thaw_hir::FfiAggregateLayout {
                field_offsets: vec![0, 8],
                field_layouts: vec![None, None],
                field_bitfields: vec![None, None],
                register_classes: vec![
                    thaw_hir::FfiRegisterClass::Integer,
                    thaw_hir::FfiRegisterClass::Sse,
                ],
                size: 16,
                alignment: 8,
                indirect: false,
            },
        )
        .unwrap();
    }
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_bit_record",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let mut invalid_bits = program.clone();
    assert!(thaw_hir::set_ffi_aggregate_layout(
        &mut invalid_bits,
        "native_bit_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 0, 0, 0, 8],
            field_layouts: vec![None, None, None, None, None],
            field_bitfields: vec![
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 32,
                    bit_width: 1,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 1,
                    bit_width: 1,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 2,
                    bit_width: 5,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 7,
                    bit_width: 6,
                    storage_bytes: 4,
                    signed: true,
                }),
                None,
            ],
            register_classes: vec![],
            size: 32,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap_err()
    .contains("invalid bit offset"));
    thaw_hir::set_ffi_aggregate_layout(
        &mut program,
        "native_bit_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 0, 0, 0, 8],
            field_layouts: vec![None, None, None, None, None],
            field_bitfields: vec![
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 0,
                    bit_width: 1,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 1,
                    bit_width: 1,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 2,
                    bit_width: 5,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 7,
                    bit_width: 6,
                    storage_bytes: 4,
                    signed: true,
                }),
                None,
            ],
            register_classes: vec![],
            size: 32,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_nested_aligned_record",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    assert!(thaw_hir::set_ffi_aggregate_layout(
        &mut program.clone(),
        "native_nested_aligned_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 48],
            field_layouts: vec![None, None],
            field_bitfields: vec![None, None],
            register_classes: vec![],
            size: 64,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap_err()
    .contains("needs a nested field layout"));
    thaw_hir::set_ffi_aggregate_layout(
        &mut program,
        "native_nested_aligned_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 48],
            field_layouts: vec![
                Some(Box::new(thaw_hir::FfiAggregateLayout {
                    field_offsets: vec![0, 16],
                    field_layouts: vec![None, None],
                    field_bitfields: vec![None, None],
                    register_classes: vec![],
                    size: 32,
                    alignment: 32,
                    indirect: false,
                })),
                None,
            ],
            field_bitfields: vec![None, None],
            register_classes: vec![],
            size: 64,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap();
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_checked_aligned_record",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_checked_aligned_record",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    thaw_hir::set_ffi_aggregate_layout(
        &mut program,
        "native_checked_aligned_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 16],
            field_layouts: vec![None, None],
            field_bitfields: vec![None, None],
            register_classes: vec![],
            size: 32,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_explicit_aggregate_layout");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.module.print_to_string().to_string();
    assert!(ir.contains("alloca <{ i8, [15 x i8], double, [8 x i8] }>, align 32"));
    assert!(ir.contains("declare { i64, double } @native_register_record()"));

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-explicit-layout-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stddef.h>\n\
         typedef struct __attribute__((aligned(32))) { _Bool active; char padding[15]; double value; char tail[8]; } AlignedRecord;\n\
         typedef struct { AlignedRecord value; const char *error; } AlignedRecordResult;\n\
         typedef struct __attribute__((aligned(32))) { AlignedRecord meta; char padding[16]; double total; char tail[8]; } NestedAlignedRecord;\n\
         typedef struct __attribute__((aligned(32))) { unsigned active:1; unsigned ready:1; unsigned count:5; signed delta:6; double value; } BitRecord;\n\
         typedef struct { _Bool active; double value; } RegisterRecord;\n\
         typedef struct { RegisterRecord value; const char *error; } RegisterRecordResult;\n\
         _Static_assert(sizeof(BitRecord) == 32, \"unexpected BitRecord size\");\n\
         _Static_assert(offsetof(BitRecord, value) == 8, \"unexpected BitRecord value offset\");\n\
         AlignedRecord native_aligned_record(void) { return (AlignedRecord){1, {0}, 42, {0}}; }\n\
         AlignedRecordResult native_checked_aligned_record(double value) {\n\
           if (value < 0) return (AlignedRecordResult){{0, {0}, 0, {0}}, \"aligned check failed\"};\n\
           return (AlignedRecordResult){{1, {0}, value * 3, {0}}, 0};\n\
         }\n\
         NestedAlignedRecord native_nested_aligned_record(void) {\n\
           return (NestedAlignedRecord){{1, {0}, 20, {0}}, {0}, 22, {0}};\n\
         }\n\
         BitRecord native_bit_record(void) { return (BitRecord){1, 0, 17, -7, 42}; }\n\
         RegisterRecord native_register_record(void) { return (RegisterRecord){1, 55}; }\n\
         RegisterRecordResult native_checked_register_record(double value) {\n\
           if (value < 0) return (RegisterRecordResult){{0, 0}, \"register check failed\"};\n\
           return (RegisterRecordResult){{1, value * 2}, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "true\n42\n21\naligned check failed\ntrue\n42\ntrue\nfalse\n17\n-7\n42\ntrue\n55\n18\nregister check failed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_object_return_copies_and_destroys_owned_string_fields() {
    let source = r#"
        declare function native_message(): { code: number; message: string };
        declare function native_checked_message(value: number): { code: number; message: string };
        declare function message_destroy_count(): number;

        function main(): void {
            const direct: { code: number; message: string } = native_message();
            const checked: { code: number; message: string } = native_checked_message(2);
            console.log(direct.code);
            console.log(direct.message);
            console.log(checked.message);
            console.log(message_destroy_count());
            try {
                native_checked_message(0 - 1);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    for symbol in ["native_message", "native_checked_message"] {
        thaw_hir::set_ffi_ownership(
            &mut program,
            symbol,
            thaw_hir::FfiOwnership::Owned {
                destroy: "destroy_message".into(),
            },
            thaw_hir::FfiOwnership::Borrowed,
        )
        .unwrap();
        thaw_hir::set_ffi_string_abi(
            &mut program,
            symbol,
            if symbol == "native_message" {
                vec![]
            } else {
                vec![thaw_hir::FfiStringAbi::NullTerminated]
            },
            thaw_hir::FfiStringAbi::NullTerminated,
            thaw_hir::FfiCallingConvention::C,
            thaw_hir::FfiAggregateAbi::Portable,
        )
        .unwrap();
    }
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_checked_message",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_owned_object_fields");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-owned-object-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdlib.h>\n#include <string.h>\n\
         typedef struct { double code; char *message; } Message;\n\
         typedef struct { Message value; const char *error; } MessageResult;\n\
         static int destroys;\n\
         static char *copy(const char *s) { size_t n = strlen(s) + 1; char *p = malloc(n); memcpy(p, s, n); return p; }\n\
         Message native_message(void) { return (Message){1, copy(\"direct message\")}; }\n\
         MessageResult native_checked_message(double value) {\n\
           if (value < 0) return (MessageResult){{0, 0}, \"checked message failed\"};\n\
           return (MessageResult){{value, copy(\"checked message\")}, 0};\n\
         }\n\
         void destroy_message(char *p) { ++destroys; free(p); }\n\
         double message_destroy_count(void) { return destroys; }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "1\ndirect message\nchecked message\n2\nchecked message failed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_nested_aggregate_returns_are_rebuilt_and_owned_recursively() {
    let source = r#"
        declare function native_nested(): { meta: { label: string; ok: boolean }; values: number[] };
        declare function native_checked_nested(value: number): { meta: { label: string; ok: boolean }; values: number[] };
        declare function nested_destroy_count(): number;

        function main(): void {
            const direct = native_nested();
            console.log(direct.meta.label);
            console.log(direct.meta.ok);
            console.log(direct.values[0] + direct.values[1]);
            console.log(nested_destroy_count());
            const checked = native_checked_nested(3);
            console.log(checked.meta.label);
            console.log(checked.values[0]);
            console.log(nested_destroy_count());
            try {
                native_checked_nested(0 - 1);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    for symbol in ["native_nested", "native_checked_nested"] {
        thaw_hir::set_ffi_ownership(
            &mut program,
            symbol,
            thaw_hir::FfiOwnership::Owned {
                destroy: "destroy_nested_leaf".into(),
            },
            thaw_hir::FfiOwnership::Borrowed,
        )
        .unwrap();
        thaw_hir::set_ffi_string_abi(
            &mut program,
            symbol,
            if symbol == "native_nested" {
                vec![]
            } else {
                vec![thaw_hir::FfiStringAbi::NullTerminated]
            },
            thaw_hir::FfiStringAbi::NullTerminated,
            thaw_hir::FfiCallingConvention::C,
            thaw_hir::FfiAggregateAbi::Portable,
        )
        .unwrap();
    }
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_checked_nested",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_nested_aggregate");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-nested-aggregate-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n#include <stdlib.h>\n#include <string.h>\n\
         typedef struct { double *data; int64_t len; } NumberArray;\n\
         typedef struct { char *label; _Bool ok; } Meta;\n\
         typedef struct { Meta meta; NumberArray values; } Nested;\n\
         typedef struct { Nested value; const char *error; } NestedResult;\n\
         static int destroys;\n\
         static char *copy(const char *s) { size_t n = strlen(s) + 1; char *p = malloc(n); memcpy(p, s, n); return p; }\n\
         static Nested make_nested(const char *label, double first) {\n\
           double *values = malloc(2 * sizeof(double)); values[0] = first; values[1] = 5;\n\
           return (Nested){{copy(label), 1}, {values, 2}};\n\
         }\n\
         Nested native_nested(void) { return make_nested(\"direct nested\", 2); }\n\
         NestedResult native_checked_nested(double value) {\n\
           if (value < 0) return (NestedResult){{{0, 0}, {0, 0}}, \"nested check failed\"};\n\
           return (NestedResult){make_nested(\"checked nested\", value), 0};\n\
         }\n\
         void destroy_nested_leaf(void *p) { ++destroys; free(p); }\n\
         double nested_destroy_count(void) { return destroys; }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "direct nested\ntrue\n7\n2\nchecked nested\n3\n4\nnested check failed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_supports_pointer_length_string_parameters_and_returns() {
    let source = r#"
        declare function native_slice(value: string): string;

        function main(): void {
            console.log(native_slice("hello"));
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_slice",
        vec![thaw_hir::FfiStringAbi::PointerLength],
        thaw_hir::FfiStringAbi::PointerLength,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Internal,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_string_slice");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-string-slice-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         typedef struct { const char *data; int64_t len; } ThawStringSlice;\n\
         static const char result[] = {'O', 'K', '!'};\n\
         ThawStringSlice native_slice(const char *value, int64_t len) {\n\
           return value[0] == 'h' && len == 5 ? (ThawStringSlice){result, 3} : (ThawStringSlice){result, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "OK!\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// docs/design/bridge.md section 5's marshal-adapter gap, closed:
/// unlike the plain-`number` case above, a real C function taking an
/// array almost never expects Thaw's own internal `[i64 len][f64
/// elements...]` buffer -- it expects the near-universal `(const
/// double*, int64_t len)` two-argument convention instead, exactly
/// what this real (not Thaw-authored) C function below declares. If
/// `ffi_param_types`/`compile_ffi_call` still passed Thaw's raw buffer
/// pointer as a single argument, this would either fail to link
/// (arity mismatch caught by `cc`) or silently misread memory as a
/// `(double*, int64_t)` pair -- it does neither: the sum comes back
/// correct.
#[test]
fn ffi_call_marshals_a_number_array_into_pointer_plus_length() {
    let source = r#"
        declare function native_sum(xs: number[]): number;

        function main(): void {
            const xs: number[] = [1, 2, 3, 4];
            console.log(native_sum(xs));
        }
    "#;

    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    assert_eq!(
        program.extern_functions.len(),
        1,
        "sanity: this is really an FFI call"
    );

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_array_marshal");
    compiler.compile_program(&program).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi_array_marshal-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");

    compiler.write_object_file(&obj_path).unwrap();

    // A real, independently-written C function -- not one shaped
    // around Thaw's own array layout. `len` is genuinely used (not
    // just accepted and ignored), so a wrong length would also
    // produce a wrong sum, not just "happen to work".
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         double native_sum(const double* xs, int64_t len) {\n\
         \x20\x20double total = 0;\n\
         \x20\x20for (int64_t i = 0; i < len; i++) { total += xs[i]; }\n\
         \x20\x20return total;\n\
         }\n",
    )
    .unwrap();
    let cc_status = Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .expect("failed to invoke `cc` to build the native stand-in library");
    assert!(cc_status.success(), "compiling native.c failed");

    let arena_lib = build_staticlib("thaw-arena");
    let link_status = Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    let output = Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    assert!(output.status.success(), "binary exited non-zero");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "10\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ffi_call_marshals_string_arrays_in_both_directions() {
    let source = r#"
        declare function native_labels(values: string[]): string[];

        function main(): void {
            const result: string[] = native_labels(["first", "second"]);
            console.log(result[0], result[1], result.length);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_labels",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_string_arrays");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-string-arrays-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         #include <string.h>\n\
         typedef struct { const char **data; int64_t len; } ThawStringArray;\n\
         static const char *result[] = {\"accepted\", \"strings\"};\n\
         ThawStringArray native_labels(const char **values, int64_t len) {\n\
           int valid = len == 2 && strcmp(values[0], \"first\") == 0 && strcmp(values[1], \"second\") == 0;\n\
           return valid ? (ThawStringArray){result, 2} : (ThawStringArray){result, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "accepted strings 2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_boolean_arrays_in_both_directions() {
    let source = r#"
        declare function native_flags(values: boolean[]): boolean[];

        function main(): void {
            const result: boolean[] = native_flags([true, false, true]);
            console.log(result[0], result[1], result[2], result.length);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_flags",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_boolean_arrays");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-boolean-arrays-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         typedef struct { const uint8_t *data; int64_t len; } ThawBoolArray;\n\
         static const uint8_t result[] = {0, 1, 0};\n\
         ThawBoolArray native_flags(const uint8_t *values, int64_t len) {\n\
           int valid = len == 3 && values[0] == 1 && values[1] == 0 && values[2] == 1;\n\
           return valid ? (ThawBoolArray){result, 3} : (ThawBoolArray){result, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "false true false 3\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_owned_boolean_array_return_is_copied_and_destroyed() {
    let source = r#"
        declare function native_owned_flags(): boolean[];
        declare function flag_destroy_count(): number;

        function main(): void {
            const flags: boolean[] = native_owned_flags();
            console.log(flags[0], flags[1], flags.length, flag_destroy_count());
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_ownership(
        &mut program,
        "native_owned_flags",
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_flags".into(),
        },
        thaw_hir::FfiOwnership::Borrowed,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_owned_flags",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_owned_boolean_array");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-owned-bool-array-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        r#"#include <stdint.h>
#include <stdlib.h>
typedef struct { uint8_t *data; int64_t len; } BoolArray;
static int destroyed = 0;
BoolArray native_owned_flags(void) {
  uint8_t *data = malloc(2);
  data[0] = 1;
  data[1] = 0;
  return (BoolArray){data, 2};
}
void destroy_flags(void *data) { destroyed += 1; free(data); }
double flag_destroy_count(void) { return destroyed; }
"#,
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "true false 2 1\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_js_value_arrays_in_both_directions() {
    let source = r#"
        declare function native_handle(value: number): JsValue;
        declare function native_handles(values: JsValue[]): JsValue[];

        function main(): void {
            const input: JsValue[] = [native_handle(11), native_handle(22)];
            console.log(native_handles(input).length);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_handles",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_js_value_arrays");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-js-value-arrays-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         typedef struct { const uint64_t *data; int64_t len; } ThawHandleArray;\n\
         static const uint64_t result[] = {33, 44, 55};\n\
         uint64_t native_handle(double value) { return (uint64_t)value; }\n\
         ThawHandleArray native_handles(const uint64_t *values, int64_t len) {\n\
           int valid = len == 2 && values[0] == 11 && values[1] == 22;\n\
           return valid ? (ThawHandleArray){result, 3} : (ThawHandleArray){result, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_recursively_marshals_array_fields_in_object_parameters() {
    let source = r#"
        declare function native_bundle(value: {
            label: string;
            numbers: number[];
            flags: boolean[];
            nested: { names: string[] };
        }): number;

        function main(): void {
            console.log(native_bundle({
                label: "bundle",
                numbers: [1, 2],
                flags: [true, false],
                nested: { names: ["left", "right"] }
            }));
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_nested_object_arrays");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-nested-object-arrays-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         #include <string.h>\n\
         double native_bundle(\n\
           const char *label,\n\
           const double *numbers, int64_t number_len,\n\
           const uint8_t *flags, int64_t flag_len,\n\
           const char **names, int64_t name_len\n\
         ) {\n\
           return strcmp(label, \"bundle\") == 0 &&\n\
             number_len == 2 && numbers[0] == 1 && numbers[1] == 2 &&\n\
             flag_len == 2 && flags[0] == 1 && flags[1] == 0 &&\n\
             name_len == 2 && strcmp(names[0], \"left\") == 0 && strcmp(names[1], \"right\") == 0\n\
             ? 42 : 0;\n\
         }\n",
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Same gap, for `Object` params: a real C function is far more
/// likely to be a flat multi-argument function than to agree with
/// Thaw's own arena struct layout, so an object-typed FFI parameter
/// is flattened into one scalar C argument per field, in declared
/// order. `x*10 + y` (rather than something field-order-symmetric
/// like `x + y`) specifically catches a field-order bug: if `x`/`y`
/// were swapped, `{x: 3, y: 4}` would produce `43` instead of `34`.
#[test]
fn ffi_call_recursively_marshals_tagged_fixed_parameters() {
    let source = r#"
        declare function native_tagged(
            optional: number | undefined,
            nullable: number | null,
            nullish: number | null | undefined,
            object: {
                optional: number | undefined;
                nullable: number | null;
                nullish: number | null | undefined;
            }
        ): number;

        function main(): void {
            console.log(native_tagged(
                undefined,
                null,
                3,
                { optional: 4, nullable: 5, nullish: undefined }
            ));
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_tagged_fixed_parameters");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-tagged-fixed-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        r#"#include <stdint.h>
double native_tagged(
  uint8_t optional_tag, double optional,
  uint8_t nullable_tag, double nullable,
  uint8_t nullish_tag, double nullish,
  uint8_t object_optional_tag, double object_optional,
  uint8_t object_nullable_tag, double object_nullable,
  uint8_t object_nullish_tag, double object_nullish
) {
  return optional_tag == 0 &&
    nullable_tag == 0 &&
    nullish_tag == 0 && nullish == 3 &&
    object_optional_tag == 1 && object_optional == 4 &&
    object_nullable_tag == 1 && object_nullable == 5 &&
    object_nullish_tag == 2
    ? 42 : 0;
}
"#,
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_restores_portable_tagged_returns() {
    let source = r#"
        declare function native_optional(present: boolean): number | undefined;
        declare function native_nullable(present: boolean): number | null;
        declare function native_nullish(state: number): number | null | undefined;

        function main(): void {
            console.log(native_optional(true), native_optional(false));
            console.log(native_nullable(true), native_nullable(false));
            console.log(native_nullish(0), native_nullish(1), native_nullish(2));
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    for symbol in ["native_optional", "native_nullable", "native_nullish"] {
        thaw_hir::set_ffi_string_abi(
            &mut program,
            symbol,
            vec![thaw_hir::FfiStringAbi::NullTerminated],
            thaw_hir::FfiStringAbi::NullTerminated,
            thaw_hir::FfiCallingConvention::C,
            thaw_hir::FfiAggregateAbi::Portable,
        )
        .unwrap();
    }
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_portable_tagged_returns");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-tagged-returns-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        r#"#include <stdint.h>
typedef struct { uint8_t tag; double value; } TaggedNumber;
TaggedNumber native_optional(uint8_t present) {
  return (TaggedNumber){present, 21};
}
TaggedNumber native_nullable(uint8_t present) {
  return (TaggedNumber){present, 22};
}
TaggedNumber native_nullish(double state) {
  return (TaggedNumber){(uint8_t)state, 23};
}
"#,
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "21 undefined\n22 null\n23 null undefined\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_portable_tuples_in_both_directions() {
    let source = r#"
        declare function native_tuple(value: [number, string, boolean]): [string, number, boolean];

        function main(): void {
            const result: [string, number, boolean] = native_tuple([1, "two", true]);
            console.log(result[0], result[1], result[2]);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_tuple",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_portable_tuples");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-tuples-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        r#"#include <stdbool.h>
#include <string.h>
typedef struct { const char *text; double number; bool flag; } NativeTuple;
NativeTuple native_tuple(double number, const char *text, bool flag) {
  static const char result[] = "tuple";
  return number == 1 && strcmp(text, "two") == 0 && flag
    ? (NativeTuple){result, 24, false}
    : (NativeTuple){result, 0, true};
}
"#,
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "tuple 24 false\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_owned_tuple_return_releases_each_owned_leaf() {
    let source = r#"
        declare function native_owned_tuple(): [string, boolean[]];
        declare function tuple_destroy_count(): number;

        function main(): void {
            const value: [string, boolean[]] = native_owned_tuple();
            console.log(value[0], value[1][0], value[1][1], tuple_destroy_count());
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_ownership(
        &mut program,
        "native_owned_tuple",
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_tuple_leaf".into(),
        },
        thaw_hir::FfiOwnership::Borrowed,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_owned_tuple",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_owned_tuple");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-owned-tuple-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        r#"#include <stdint.h>
#include <stdlib.h>
#include <string.h>
typedef struct { uint8_t *data; int64_t len; } BoolArray;
typedef struct { char *text; BoolArray flags; } OwnedTuple;
static int destroyed = 0;
OwnedTuple native_owned_tuple(void) {
  char *text = malloc(6);
  memcpy(text, "owned", 6);
  uint8_t *flags = malloc(2);
  flags[0] = 1;
  flags[1] = 0;
  return (OwnedTuple){text, {flags, 2}};
}
void destroy_tuple_leaf(void *value) { destroyed += 1; free(value); }
double tuple_destroy_count(void) { return destroyed; }
"#,
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
    let arena_lib = build_staticlib("thaw-arena");
    assert!(Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "owned true false 2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_an_object_into_one_scalar_argument_per_field() {
    let source = r#"
        declare function native_combine(p: { x: number; y: number }): number;

        function main(): void {
            console.log(native_combine({ x: 3, y: 4 }));
        }
    "#;

    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    assert_eq!(
        program.extern_functions.len(),
        1,
        "sanity: this is really an FFI call"
    );

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_object_marshal");
    compiler.compile_program(&program).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi_object_marshal-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");

    compiler.write_object_file(&obj_path).unwrap();

    std::fs::write(
        &native_c_path,
        "double native_combine(double x, double y) { return x * 10 + y; }\n",
    )
    .unwrap();
    let cc_status = Command::new("cc")
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .expect("failed to invoke `cc` to build the native stand-in library");
    assert!(cc_status.success(), "compiling native.c failed");

    let arena_lib = build_staticlib("thaw-arena");
    let link_status = Command::new("cc")
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    let output = Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    assert!(output.status.success(), "binary exited non-zero");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "34\n");

    let _ = std::fs::remove_dir_all(&dir);
}
