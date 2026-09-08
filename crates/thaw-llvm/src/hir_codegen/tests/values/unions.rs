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

