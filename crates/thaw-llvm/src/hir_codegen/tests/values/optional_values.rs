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

