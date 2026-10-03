#[test]
fn fixed_object_values_entries_skip_only_runtime_hidden_markers() {
    let source = r#"
        class Leaf { value: number; label: string; constructor() { this.value = 3; this.label = "s"; } }
        const marker = "__thaw_class_identity_\u001eLeaf";
        let reads = 0;
        interface Mixed { first: number; second: string; }
        const mixed: Mixed = {
            get first(): number { reads++; return 7; },
            second: "s",
        };
        function main(): void {
            const instance = new Leaf();
            const alias = instance;
            console.log(Object.values(alias).length, Object.values(alias).join(","));
            console.log(Object.entries(instance).length, Object.entries(instance)[0][0]);
            const ordinary = { ["__thaw_class_identity_\u001eLeaf"]: true, get value(): number { reads++; return 5; } };
            console.log(Object.values(ordinary).length, Object.entries(ordinary).length);
            console.log(Object.entries(ordinary)[0][0] === marker);
            const values = Object.values(mixed);
            const entries = Object.entries(mixed);
            console.log(values.length, values[0], values[1], entries.length, entries[0][0], entries[0][1], reads);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "visible_fixed_values_entries"),
        "2 3,s\n2 value\n2 2\ntrue\n2 7 s 2 first 7 5\n"
    );
}

#[test]
fn hides_only_registered_class_markers_in_read_only_object_queries() {
    let source = r#"
        class Leaf { value: number; constructor() { this.value = 3; } }
        function main(): void {
            const instance = new Leaf();
            const alias = instance;
            const marker = "__thaw_class_identity_\u001eLeaf";
            console.log(Object.keys(alias).join(","));
            console.log(Object.getOwnPropertyNames(instance).join(","));
            console.log(Reflect.ownKeys(instance).join(","));
            console.log(Object.hasOwn(instance, marker), instance.hasOwnProperty(marker), Reflect.has(instance, marker));
            console.log(Object.getOwnPropertyDescriptor(instance, "__thaw_class_identity_\u001eLeaf") === undefined);
            console.log(JSON.stringify(instance));
            const ordinary = { ["__thaw_class_identity_\u001eLeaf"]: true, value: 5 };
            console.log(Object.keys(ordinary).length, Object.hasOwn(ordinary, marker));
            console.log(Object.getOwnPropertyDescriptor(ordinary, "__thaw_class_identity_\u001eLeaf") !== undefined);
            console.log(JSON.stringify(ordinary).includes("Leaf"));
            const aggregate = new AggregateError([1], "x");
            console.log(Object.keys(aggregate).join(","));
            const suppressed = new SuppressedError("a", "b", "x");
            console.log(Object.keys(suppressed).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "hidden_class_marker_read_only_queries"),
        "value\nvalue\nvalue\nfalse false false\ntrue\n{\"value\":3}\n2 true\ntrue\ntrue\nmessage,name,errors\nmessage,name,error,suppressed\n"
    );
}

#[test]
fn compiles_omittable_arguments_beyond_one_mask_word() {
    let required = (0..64)
        .map(|index| format!("p{index}: number"))
        .collect::<Vec<_>>()
        .join(", ");
    let args = vec!["1"; 64].join(", ");
    let source = format!(
        "function wide({required}, last: number = 7): number {{ return last; }} \
         function main(): void {{ \
         console.log(wide({args}), wide({args}, undefined)); \
         const callable: ({required}, last?: number) => number = wide; \
         console.log(callable({args})); }}"
    );
    assert_eq!(compile_and_run(&source, "wide_optional_function"), "7 7\n7\n");

    let method_required = (0..63)
        .map(|index| format!("p{index}: number"))
        .collect::<Vec<_>>()
        .join(", ");
    let method_args = vec!["1"; 63].join(", ");
    let source = format!(
        "let defaultRuns = 0; \
         function fallback(): number {{ defaultRuns++; return 7; }} \
         class Base {{ value: number; \
         constructor({method_required}, last: number = fallback()) {{ this.value = last; }} \
         method({method_required}, last: number = fallback()): number {{ return last + 2; }} }} \
         class Middle extends Base {{}} class Derived extends Middle {{}} \
         function main(): void {{ \
         const first = new Derived({method_args}); \
         console.log(first.value, first.method({method_args}), defaultRuns); \
         const second = new Derived({method_args}, undefined); \
         console.log(second.value, second.method({method_args}, undefined), defaultRuns); }}"
    );
    assert_eq!(compile_and_run(&source, "wide_optional_class"), "7 9 2\n7 9 4\n");
}

#[test]
fn compiles_classic_for_loop_over_a_number_array() {
    let source = r#"
        function main(): void {
            const xs: number[] = [10, 20, 30, 40];
            let sum: number = 0;
            for (let i = 0; i < xs.length; i = i + 1) {
                sum = sum + xs[i];
            }
            console.log(sum);
        }
    "#;
    assert_eq!(compile_and_run(source, "forloop"), "100\n");
}

#[test]
fn compiles_static_computed_object_and_json_access() {
    let source = r#"
        async function asyncPoint(point: { value: number }): Promise<{ value: number }> {
            await sleep(1);
            console.log("object");
            return point;
        }
        async function main(): Promise<void> {
            let point = { value: 1 };
            console.log(point["value"]);
            point["value"] = 2;
            point["value"] += 3;
            console.log(point["value"]++);
            console.log(point.value);
            console.log((await asyncPoint(point))["value"]--);
            console.log(point.value);
            const data: Json = JSON.parse("{\"name\":\"thaw\"}");
            console.log(String(data["name"]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "static_computed_properties"),
        "1\n5\n6\nobject\n6\n5\nthaw\n"
    );
}

#[test]
fn compiles_union_array_metadata_through_native_methods() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        function print(values: Result[]): void {
            for (const item of values) {
                if (item.kind === "number") console.log(item.value + 1);
                else console.log(item.value + "!");
            }
        }
        function main(): void {
            const results: Result[] = [
                { kind: "number", value: 6 },
                { kind: "text", value: "sync" }
            ];
            const copied = results
                .slice(0, 2)
                .concat(results.slice(0, 0))
                .copyWithin(0, 0);
            const filtered = results
                .filter(() => true)
                .toReversed()
                .reverse();
            const replaced = results
                .toSpliced(1, 0)
                .with(0, results[0]);
            print(copied);
            print(filtered);
            print(replaced);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "union_array_method_metadata"),
        "7\nsync!\n7\nsync!\n7\nsync!\n"
    );
}

#[test]
fn compiles_primitive_abstract_equality_with_ordered_awaits() {
    let source = r#"
        function number(label: string, value: number): number {
            console.log(label);
            return value;
        }
        function text(label: string, value: string): string {
            console.log(label);
            return value;
        }
        async function delayed(value: string): Promise<string> {
            await sleep(1);
            console.log("awaited-equality");
            return value;
        }
        async function main(): Promise<void> {
            console.log(text("left", "1") == number("right", 1));
            console.log("" == 0);
            console.log("bad" != 0);
            console.log(true == 1);
            console.log(false == "0");
            console.log("NaN" == (0 / 0));
            console.log((await delayed("42")) == 42);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "primitive_abstract_equality"),
        "left\nright\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\nawaited-equality\ntrue\n"
    );
}

#[test]
fn compiles_native_to_string_methods_and_awaited_receivers() {
    let source = r#"
        function objectValue(): { value: number } {
            console.log("object-method-receiver");
            return { value: 1 };
        }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            console.log("awaited-method-receiver");
            return value;
        }
        async function main(): Promise<void> {
            console.log((42.5).toString());
            console.log(true.toString());
            console.log("word".toString());
            const numbers: number[] = [1, 2.5];
            console.log(numbers.toString());
            const tuple: [number, string, boolean] = [3, "x", false];
            console.log(tuple.toString());
            console.log(objectValue().toString());
            console.log((await delayed(9)).toString());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_to_string_methods"),
        "42.5\ntrue\nword\n1,2.5\n3,x,false\nobject-method-receiver\n[object Object]\nawaited-method-receiver\n9\n"
    );
}

#[test]
fn preserves_union_discriminants_for_element_returning_array_methods() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        function main(): void {
            const results: Result[] = [
                { kind: "number", value: 1 },
                { kind: "text", value: "found" }
            ];
            const at = results.at(0);
            if (at !== undefined) print(at);
            const found = results.find(item => item.kind === "text");
            if (found !== undefined) print(found);
            const last = results.findLast(() => true);
            if (last !== undefined) print(last);
            const reduced = results.reduce((previous, current) =>
                current.kind === "text" ? current : previous
            );
            print(reduced);
            const initial: Result = { kind: "number", value: 2 };
            const reducedRight = results.reduceRight(previous => previous, initial);
            print(reducedRight);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "union_array_element_metadata"),
        "11\nfound!\nfound!\nfound!\n12\n"
    );
}

#[test]
fn compiles_optional_native_method_calls() {
    let source = r#"
        interface Handler { run: (value: number) => number; }
        function text(present: boolean): string | undefined {
            if (present) return "ok";
            return undefined;
        }
        function values(present: boolean): number[] | undefined {
            if (present) return [1, 2, 3];
            return undefined;
        }
        function maybeHandler(present: boolean): Handler | undefined {
            if (present) return { run: (value: number) => value * 3 };
            return undefined;
        }
        function needle(): number {
            console.log("needle");
            return 2;
        }
        async function delayedValues(): Promise<number[] | undefined> {
            await sleep(1);
            return [4, 5];
        }
        async function main(): Promise<void> {
            console.log(text(true)?.toUpperCase());
            console.log(text(false)?.toUpperCase());
            console.log(values(true)?.includes(needle()));
            console.log(values(false)?.includes(needle()));
            console.log(maybeHandler(true)?.run(needle()));
            console.log(maybeHandler(false)?.run(needle()));
            console.log(values(true)?.at(9));
            console.log(values(false)?.at(0));
            console.log(values(false)?.forEach(value => { console.log(value); }));
            console.log(values(true)?.forEach(value => { console.log(value); }));
            console.log((await delayedValues())?.includes(5));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_native_methods"),
        "OK\nundefined\nneedle\ntrue\nundefined\nneedle\n6\nundefined\nundefined\nundefined\nundefined\n1\n2\n3\nundefined\ntrue\n"
    );
}

#[test]
fn supports_generic_interface_method_signatures() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        interface Service<T> {
            load(): T[][];
        }
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        function main(): void {
            const service: Service<Result> = {
                load: (): Result[][] => [[{ kind: "text", value: "generic" }]],
            };
            print(service.load().flat()[0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_interface_method_signature"),
        "generic!\n"
    );
}

#[test]
fn supports_object_type_method_signatures() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        function main(): void {
            const service: { load(): Result[][] } = {
                load: (): Result[][] => [[{ kind: "number", value: 2 }]],
            };
            print(service.load().flat()[0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_type_method_signature"),
        "12\n"
    );
}

#[test]
fn compiles_utf16_string_search_methods() {
    let source = r#"
        function text(): string {
            console.log("receiver");
            return "abcabc";
        }
        function needle(): string {
            console.log("needle");
            return "bc";
        }
        function position(): string {
            console.log("position");
            return "2";
        }
        async function delayedText(): Promise<string> {
            await sleep(1);
            console.log("awaited-text");
            return "thaw-runtime";
        }
        async function delayedNeedle(): Promise<string> {
            console.log("awaited-needle");
            await sleep(1);
            return "bc";
        }
        async function main(): Promise<void> {
            const unicode: string = "😀a😀";
            console.log(unicode.indexOf("a"));
            console.log(unicode.indexOf("😀", 1));
            console.log(unicode.includes("a", -20));
            console.log(unicode.startsWith("a", 2));
            console.log(unicode.endsWith("😀", 2));
            console.log(unicode.endsWith("😀"));
            console.log(unicode.indexOf("", 99));
            console.log(unicode.startsWith("", 99));
            console.log(unicode.endsWith("", -1));
            console.log("123".includes(2));
            console.log(text().indexOf(needle(), position()));
            console.log(unicode.lastIndexOf("😀"));
            console.log(unicode.lastIndexOf("😀", 2));
            console.log(unicode.lastIndexOf("😀", -1));
            console.log(unicode.lastIndexOf("", 99));
            console.log(unicode.lastIndexOf("", 0 / 0));
            console.log(text().lastIndexOf(await delayedNeedle(), "99"));
            console.log((await delayedText()).startsWith("thaw"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_search"),
        "2\n3\ntrue\ntrue\ntrue\ntrue\n5\ntrue\ntrue\ntrue\nreceiver\nneedle\nposition\n4\n3\n0\n0\n5\n0\nreceiver\nawaited-needle\n4\nawaited-text\ntrue\n"
    );
}

#[test]
fn compiles_and_runs_native_class_constructor_fields() {
    let source = r#"
        class Counter {
            value: number = 1;
            label: string;
            constructor(value: number, label: string) {
                this.value = value;
                this.label = label;
            }
        }
        function main(): void {
            const counter = new Counter(42, "ready");
            console.log(counter.value);
            console.log(counter.label);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_class_constructor_fields"),
        "42\nready\n"
    );
}

#[test]
fn compiles_class_fields_with_types_inferred_from_literal_initializers() {
    let source = r#"
        class Widget {
            name = "widget";
            active = true;
            count = 0;
            static #instances = 0;
            static label = "static-widget";
            constructor() {
                Widget.#instances++;
            }
            describe(): string {
                return this.name + " " + this.active + " " + this.count;
            }
            static instanceCount(): number {
                return Widget.#instances;
            }
        }
        function main(): void {
            const w = new Widget();
            console.log(w.describe());
            new Widget();
            console.log(Widget.instanceCount());
            console.log(Widget.label);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "class_fields_inferred_from_literals"),
        "widget true 0\n2\nstatic-widget\n"
    );
}

#[test]
fn compiles_class_fields_inferred_from_aggregate_literals() {
    let source = r#"
        class Store {
            values = [10, 20];
            config = {
                label: "store",
                flags: [true, false],
                point: { x: 1, y: 2 }
            };
            #privateData = { names: ["a", "b"] };
            static defaults = { scores: [3, 4] };

            summarize(): string {
                this.values.push(12);
                return this.config.label + " " +
                    (this.values[0] + this.values[1] + this.values[2]) + " " +
                    this.config.flags[0] + " " +
                    (this.config.point.x + this.config.point.y) + " " +
                    this.#privateData.names[1];
            }

            static total(): number {
                Store.defaults.scores.push(5);
                return Store.defaults.scores[0] +
                    Store.defaults.scores[1] +
                    Store.defaults.scores[2];
            }
        }

        function main(): void {
            const store = new Store();
            console.log(store.summarize());
            console.log(Store.total());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "class_fields_inferred_from_aggregates"),
        "store 42 true 3 b\n12\n"
    );
}

#[test]
fn compiles_and_runs_native_class_implements() {
    let source = r#"
        interface NamedValue<N, V> { name: N; value: V; }
        class Named {
            constructor(public name: string) {}
        }
        class Value extends Named implements NamedValue<string, number> {
            constructor(name: string, public value: number) { super(name); }
        }
        function main(): void {
            const value = new Value("answer", 42);
            console.log(value.name);
            console.log(value.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_class_implements"),
        "answer\n42\n"
    );
}

#[test]
fn compiles_and_runs_native_static_class_fields() {
    let source = r#"
        class Counter {
            static base: number = 40;
            static value: number = Counter.base + 2;
            static readonly label: string = "ready";
            static self: new () => Counter = this;
            static next(): number {
                Counter.value += 1;
                return Counter.value;
            }
        }
        const initial: number = Counter.value;
        function main(): void {
            console.log(initial);
            console.log(Counter.value);
            console.log(Counter.next());
            console.log(Counter.label);
            console.log(typeof Counter.self);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_static_class_fields"),
        "42\n42\n43\nready\nfunction\n"
    );
}

#[test]
fn compiles_and_runs_inherited_native_static_class_fields() {
    let source = r#"
        class Base {
            static value: number = 40;
            static readonly label: string = "shared";
        }
        class Middle extends Base {}
        class Leaf extends Middle {}
        class Override extends Base { static value: number = 10; }
        function main(): void {
            console.log(Leaf.value);
            Leaf.value = 42;
            console.log(Base.value);
            Middle.value += 1;
            console.log(Middle.value);
            console.log(Leaf.value++);
            console.log(++Middle.value);
            console.log(Leaf.label);
            console.log(Override.value);
            Override.value = 11;
            console.log(Base.value);
            console.log(Override.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "inherited_native_static_class_fields"),
        "40\n42\n43\n43\n45\nshared\n10\n45\n11\n"
    );
}

#[test]
fn compiles_and_runs_native_static_initialization_blocks() {
    let source = r#"
        let trace: string = "";
        class Counter {
            static value: number = 1;
            static { Counter.value += 40; trace += "A"; }
            static result: number = Counter.value + 1;
            static { trace += "B"; }
        }
        function main(): void {
            console.log(Counter.value);
            console.log(Counter.result);
            console.log(trace);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_static_initialization_blocks"),
        "41\n42\nAB\n"
    );
}

#[test]
fn compiles_and_runs_computed_native_class_members() {
    let source = r#"
        const prefix = "lab";
        const labelName = `${prefix}el` as const;
        const readName = ("re" + "ad") as string;
        class Box {
            ["value"]: number;
            static ["count"]: number = 40;
            constructor(value: number) { this["value"] = value; }
            ["add"](delta: number): number { return this["value"] + delta; }
            get ["current"](): number { return this["value"]; }
            set ["current"](value: number) { this["value"] = value; }
            static ["next"](): number { return ++Box["count"]; }
        }
        class NamedBox {
            [labelName]: string = "static-computed";
            [readName](): string { return this[labelName]; }
        }
        function main(): void {
            const value = new Box(40);
            console.log(value["add"](2));
            value["current"] = 41;
            console.log(value["current"]);
            console.log(Box["next"]());
            console.log(Box["count"]);
            console.log(new NamedBox()[readName]());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "computed_native_class_members"),
        "42\n41\n41\n41\nstatic-computed\n"
    );
}

#[test]
fn compiles_and_runs_numeric_native_class_members() {
    let source = r#"
        class Numbers {
            0: string = "zero";
            2n: string = "two";
            stored: number = 4;
            static 10: string = "ten";
            3(): string { return "three"; }
            get 4(): number { return this.stored; }
            set 4(value: number) { this.stored = value; }
            static 5(): string { return "five"; }
        }
        function main(): void {
            const value = new Numbers();
            console.log(value["0"], value["2"], value["3"](), value["4"]);
            value["4"] = 40;
            console.log(value["4"], Numbers["5"](), Numbers["10"]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "numeric_native_class_members"),
        "zero two three 4\n40 five ten\n"
    );
}

#[test]
fn compiles_and_runs_native_class_tuple_spreads() {
    let source = r#"
        class Calculator {
            constructor(public offset: number) {}
            sum(left: number, right: number): number {
                return this.offset + left + right;
            }
            static sum(left: number, right: number): number { return left + right; }
        }
        function main(): void {
            const constructorArgs: [number] = [1];
            const args: [number, number] = [20, 21];
            const calculator = new Calculator(...constructorArgs);
            console.log(calculator.sum(...args));
            console.log(Calculator.sum(...args));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_class_tuple_spreads"),
        "42\n41\n"
    );
}

#[test]
fn compiles_and_runs_native_super_tuple_spreads() {
    let source = r#"
        class Base {
            constructor(public left: number, public right: number) {}
            sum(left: number, right: number): number { return left + right; }
            static sum(left: number, right: number): number { return left + right; }
        }
        class Derived extends Base {
            constructor(args: [number, number]) { super(...args); }
            sumPair(args: [number, number]): number { return super.sum(...args); }
            static sumPair(args: [number, number]): number { return super.sum(...args); }
        }
        function main(): void {
            const args: [number, number] = [20, 22];
            const value = new Derived(args);
            console.log(value.left + value.right);
            console.log(value.sumPair(args));
            console.log(Derived.sumPair(args));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_super_tuple_spreads"),
        "42\n42\n42\n"
    );
}

#[test]
fn compiles_and_runs_static_block_super_members() {
    let source = r#"
        class Base {
            static value: number = 40;
            static bump(value: number): number { return value + 1; }
            static get current(): number { return Base.value; }
            static set current(value: number) { Base.value = value; }
        }
        class Derived extends Base {
            static before: number = super.current;
            static viaMethod: number = super.bump(super.value);
            static { super.current = super.value + 2; }
            static after: number = super.current;
        }
        function main(): void {
            console.log(Derived.before);
            console.log(Derived.viaMethod);
            console.log(Derived.after);
            console.log(Base.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "static_block_super_members"),
        "40\n41\n42\n42\n"
    );
}

#[test]
fn compiles_and_runs_native_instanceof() {
    let source = r#"
        let calls: number = 0;
        class Base {}
        class Middle extends Base {}
        class Leaf extends Middle {}
        class Other {}
        function make(): Leaf { calls += 1; return new Leaf(); }
        function main(): void {
            const value = new Leaf();
            console.log(value instanceof Leaf);
            console.log(value instanceof Middle);
            console.log(value instanceof Base);
            console.log(value instanceof Other);
            console.log(make() instanceof Base);
            console.log(calls);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_instanceof"),
        "true\ntrue\ntrue\nfalse\ntrue\n1\n"
    );
}

#[test]
fn native_method_receiver_keeps_actual_class_through_function_values() {
    let source = r#"
        class Base$Name {
            constructor(public value: number) {}
            read(): number { return this.value; }
        }
        class Derived extends Base$Name {}
        class Other {
            constructor(public value: number) {}
        }
        function passthrough(read: () => number): () => number { return read; }
        function main(): void {
            const method = passthrough(new Base$Name(1).read);
            const container = { method };
            console.log(container.method.call(new Derived(42)));
            console.log(container.method.apply(new Derived(43), []));
            console.log(container.method.bind(new Derived(44))());
            for (const receiver of [new Other(99), null, undefined, 7]) {
                try {
                    console.log(container.method.call(receiver));
                } catch (error) {
                    console.log((error as any).name);
                }
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_method_receiver_identity"),
        "42\n43\n44\nTypeError\nTypeError\nTypeError\nTypeError\n"
    );
}

#[test]
fn compiles_and_runs_native_class_default_parameters() {
    let source = r#"
        class Box {
            constructor(public value: number = 40, public label: string = String(value)) {}
            add(delta: number = this.value): number { return this.value + delta; }
            static sum(left: number = 20, right: number = left + 22): number {
                return left + right;
            }
            static mixed(left: number = 20, right: number): number {
                return left + right;
            }
        }
        function main(): void {
            const missing = undefined;
            const first = new Box();
            const second = new Box(41);
            const third = new Box(42, "explicit");
            const fourth = new Box(undefined, "explicit-default");
            console.log(first.value);
            console.log(first.label);
            console.log(second.label);
            console.log(third.label);
            console.log(fourth.value);
            console.log(fourth.label);
            console.log(first.add());
            console.log(first.add(undefined));
            console.log(Box.sum());
            console.log(Box.sum(21));
            console.log(Box.sum(undefined, 1));
            console.log(Box.sum(1, undefined));
            console.log(Box.sum(missing, 1));
            console.log(Box.sum(...[undefined, 1]));
            console.log(Box.mixed(undefined, 2));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_class_default_parameters"),
        "40\n40\n41\nexplicit\n40\nexplicit-default\n80\n80\n62\n64\n21\n24\n21\n21\n22\n"
    );
}

#[test]
fn compiles_inherited_and_super_native_class_default_parameters() {
    let source = r#"
        class Base {
            constructor(public value: number = 40) {}
            add(delta: number = this.value): number { return this.value + delta; }
            static sum(left: number = 20, right: number = left + 22): number {
                return left + right;
            }
        }
        class Implicit extends Base {}
        class Explicit extends Base {
            constructor() { super(...[undefined]); }
            addAgain(): number { return super.add(...[undefined]); }
            static sumAgain(): number { return super.sum(...[undefined, 1]); }
        }
        function main(): void {
            const implicit = new Implicit();
            const explicit = new Explicit();
            console.log(implicit.value);
            console.log(explicit.value);
            console.log(explicit.addAgain());
            console.log(Explicit.sumAgain());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_inherited_default_parameters"),
        "40\n40\n80\n21\n"
    );
}

#[test]
fn compiles_native_class_optional_parameters() {
    let source = r#"
        class OptionalBox {
            constructor(public value?: number) {}
            read(): number { return this.value ?? 40; }
            add(delta?: number | null): number { return (this.value ?? 40) + (delta ?? 2); }
        }
        class OptionalDerived extends OptionalBox {}
        function main(): void {
            const empty = new OptionalBox();
            const filled = new OptionalBox(5);
            const derived = new OptionalDerived();
            console.log(empty.read());
            console.log(filled.read());
            console.log(empty.add());
            console.log(filled.add(null));
            console.log(filled.add(3));
            console.log(derived.add());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_class_optional_parameters"),
        "40\n5\n42\n7\n8\n42\n"
    );
}

#[test]
fn compiles_native_class_rest_parameters() {
    let source = r#"
        class Base {
            count: number;
            first: number;
            constructor(public seed: number, ...values: number[]) {
                this.count = values.length;
                this.first = values[0] ?? 0;
            }
            size(offset: number, ...values: number[]): number {
                return offset + values.length + (values[0] ?? 0);
            }
            static total(base: number = 10, ...values: number[]): number {
                return base + values.length;
            }
            static first(...values: string[]): string {
                return values.length > 0 ? values[0] : "empty";
            }
            static any(...values: boolean[]): boolean {
                return values.length > 0 ? values[0] : false;
            }
            static async totalAsync(base: number = 10, ...values: number[]): Promise<number> {
                await sleep(1);
                return base + values.length;
            }
        }
        class Implicit extends Base {}
        class Explicit extends Base {
            constructor() { super(1, ...[2, 3]); }
            sizeAgain(): number { return super.size(1, ...[2, 3]); }
            static totalAgain(): number { return super.total(undefined, ...[4, 5]); }
        }
        async function main(): Promise<void> {
            const base = new Base(10, 20, 30);
            const spread = new Base(...[1, 2, 3, 4]);
            const implicit = new Implicit(5, 6, 7);
            const explicit = new Explicit();
            console.log(base.seed);
            console.log(base.count);
            console.log(base.first);
            console.log(spread.count);
            console.log(base.size(1, 2, 3));
            console.log(base.size(1, ...[2, 3, 4]));
            console.log(implicit.count);
            console.log(implicit.size(1, 2, 3));
            console.log(explicit.count);
            console.log(explicit.sizeAgain());
            console.log(Base.total(undefined, 1, 2));
            console.log(Explicit.totalAgain());
            console.log(Base.first());
            console.log(Base.first("ready", "ignored"));
            console.log(Base.any());
            console.log(Base.any(true, false));
            console.log(await Base.totalAsync(undefined, 1, 2, 3));
            console.log(await Implicit.totalAsync(undefined, 1));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_class_rest_parameters"),
        "10\n2\n20\n3\n5\n6\n2\n5\n2\n5\n12\n12\nempty\nready\nfalse\ntrue\n13\n11\n"
    );
}

#[test]
fn compiles_top_level_native_class_expressions() {
    let source = r#"
        function make(): Derived { return new Derived(); }
        const before = 1, Base = class InternalBase {
            constructor(public value: number = 40) {}
            add(delta: number = this.value): number { return this.value + delta; }
            clone(): InternalBase { return new InternalBase(this.value + 1); }
        }, after = 2;
        const Derived = class InternalDerived extends Base {
            static label: string = "derived";
            static make(): InternalDerived { return new InternalDerived(); }
        };
        const Anonymous = class {
            constructor(public text: string) {}
        };
        function main(): void {
            const value = make();
            const anonymous = new Anonymous("ready");
            console.log(before);
            console.log(after);
            console.log(value.value);
            console.log(value.add());
            console.log(value.clone().value);
            console.log(value instanceof Base);
            console.log(Derived.label);
            console.log(Derived.make() instanceof Derived);
            console.log(anonymous.text);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_class_expressions"),
        "1\n2\n40\n80\n41\ntrue\nderived\ntrue\nready\n"
    );
}

#[test]
fn compiles_native_private_fields_methods_and_accessors() {
    let source = r#"
        class Vault {
            #value: number = 40;
            static #count: number = 0;
            constructor(delta: number = 2) {
                this.#value += delta;
                Vault.#count++;
            }
            #double(extra: number = 0): number { return this.#value * 2 + extra; }
            get #doubled(): number { return this.#double(); }
            set #doubled(value: number) { this.#value = value / 2; }
            read(other: Vault): number { return other.#value; }
            has(other: Vault): boolean { return #value in other; }
            reveal(): number { return this.#doubled; }
            update(value: number): number { this.#doubled = value; return this.#value; }
            static #format(value: number): string { return String(value); }
            static total(): string { return Vault.#format(Vault.#count); }
            static get #current(): number { return Vault.#count; }
            static set #current(value: number) { Vault.#count = value; }
            static reset(value: number): number {
                Vault.#current = value;
                return Vault.#current;
            }
        }
        class Derived extends Vault {
            #value: string = "derived";
            own(): string { return this.#value; }
        }
        const Expression = class Expression {
            #text: string = "private expression";
            read(): string { return this.#text; }
        };
        function main(): void {
            const first = new Vault();
            const second = new Vault(3);
            const derived = new Derived();
            const expression = new Expression();
            console.log(first.read(second));
            console.log(first.has(second));
            console.log(first.reveal());
            console.log(first.update(100));
            console.log(first.reveal());
            console.log(derived.reveal());
            console.log(derived.own());
            console.log(Vault.total());
            console.log(Vault.reset(7));
            console.log(expression.read());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_private_members"),
        "43\ntrue\n84\n50\n100\n84\nderived\n3\n7\nprivate expression\n"
    );
}

/// The sibling of `compiles_native_private_fields_methods_and_accessors`'s
/// own `#value in other` check (line ~906 above) -- that one's `other:
/// Vault` is a concretely-typed receiver, so the private-brand rewrite
/// (`#count in other` -> the mangled-field-name string check,
/// `classes/normalization.rs`'s `PrivateMemberNormalizer`) already
/// worked. This one exercises the real motivating case: a bare `object`
/// keyword receiver, callable with two structurally different shapes
/// (a real `Counter` instance and a plain empty literal). `object`
/// lowers to `HirType::Json` (`type_resolution.rs`, not `HirType::
/// Dynamic` -- `Dynamic` has no codegen representation at all, and
/// collapses into the same placeholder value an unannotated parameter
/// mid-inference uses, which is what made this fail before: `object`'s
/// permanently-dynamic *value* looked, to the "conflicting inferred
/// types" convergence loop, exactly like a not-yet-resolved parameter
/// specialized from the *first* call site, hard-erroring on the
/// second). `#count in other`'s rewritten form already runs correctly
/// against a `Json`-typed receiver (`__thaw_json_has_own`, an existing,
/// genuinely dynamic runtime lookup) with no further change needed.
#[test]
fn private_brand_check_accepts_a_bare_object_typed_receiver() {
    let source = r#"
        class Counter {
            #count: number = 0;
            hasCount(other: object): boolean {
                return #count in other;
            }
        }
        function main(): void {
            const counter = new Counter();
            console.log(counter.hasCount(counter), counter.hasCount({}));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "private_brand_check_object_receiver"),
        "true false\n"
    );
}

#[test]
fn compiles_native_abstract_classes_and_implementations() {
    let source = r#"
        abstract class Shape {
            constructor(public scale: number = 2) {}
            abstract name: string;
            abstract area(multiplier?: number): number;
            abstract get label(): string;
            describe(): string { return String(this.area()); }
            fieldName(): string { return this.name; }
        }
        abstract class Deferred extends Shape {}
        class Square extends Shape {
            name: string = "square-field";
            constructor(public side: number = 3) { super(); }
            area(multiplier?: number): number {
                return this.side * this.side * this.scale * (multiplier ?? 1);
            }
            get label(): string { return "square"; }
        }
        class Concrete extends Deferred {
            name: string = "concrete-field";
            area(multiplier?: number): number { return this.scale * (multiplier ?? 1); }
            get label(): string { return "concrete"; }
        }
        function main(): void {
            const square = new Square();
            const concrete = new Concrete();
            console.log(square.label);
            console.log(square.fieldName());
            console.log(square.describe());
            console.log(square.area(2));
            console.log(concrete.describe());
            console.log(concrete.fieldName());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_abstract_classes"),
        "square\nsquare-field\n18\n36\n2\nconcrete-field\n"
    );
}

#[test]
fn compiles_virtual_dispatch_in_inherited_native_methods() {
    let source = r#"
        class Base {
            value(): number { return 1; }
            get label(): string { return "base"; }
            read(): number { return this.value(); }
            readLabel(): string { return this.label; }
            async readAsync(): Promise<number> {
                await sleep(1);
                return this.value();
            }
        }
        class Derived extends Base {
            value(): number { return super.value() + 1; }
            get label(): string { return "derived"; }
        }
        class Leaf extends Derived {
            get label(): string { return "leaf"; }
        }
        async function main(): Promise<void> {
            const derived = new Derived();
            const leaf = new Leaf();
            console.log(derived.read());
            console.log(derived.readLabel());
            console.log(await derived.readAsync());
            console.log(leaf.read());
            console.log(leaf.readLabel());
            console.log(await leaf.readAsync());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_virtual_dispatch"),
        "2\nderived\n2\n2\nleaf\n2\n"
    );
}

#[test]
fn compiles_uninitialized_optional_native_class_fields() {
    let source = r#"
        class State {
            value?: number;
            label: string | undefined;
            #secret?: string;
            static count?: number;
            static title: string | undefined;
            read(): number { return this.value ?? 40; }
            readLabel(): string { return this.label ?? "missing"; }
            readSecret(): string { return this.#secret ?? "private-missing"; }
            set(value: number, label: string, secret: string): void {
                this.value = value;
                this.label = label;
                this.#secret = secret;
            }
        }
        class Derived extends State {}
        function main(): void {
            const state = new State();
            const derived = new Derived();
            console.log(state.read());
            console.log(state.readLabel());
            console.log(state.readSecret());
            console.log(State.count ?? 41);
            console.log(Derived.title ?? "untitled");
            state.set(2, "ready", "private-ready");
            State.count = 3;
            Derived.title = "shared";
            console.log(state.read());
            console.log(state.readLabel());
            console.log(state.readSecret());
            console.log(Derived.count ?? 0);
            console.log(State.title ?? "missing");
            console.log(derived.read());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_uninitialized_optional_fields"),
        "40\nmissing\nprivate-missing\n41\nuntitled\n2\nready\nprivate-ready\n3\nshared\n40\n"
    );
}

#[test]
fn compiles_explicitly_specialized_native_generic_classes() {
    let source = r#"
        class Box<T> {
            constructor(public value: T) {}
            get(): T { return this.value; }
            replace(value: T): T { this.value = value; return this.value; }
        }
        class Pair<T, U> {
            constructor(public first: T, public second: U) {}
            left(): T { return this.first; }
            right(): U { return this.second; }
        }
        class NumberBox extends Box<number> {
            double(): number { return this.value * 2; }
        }
        function read(value: Box<number>): number { return value.get(); }
        function main(): void {
            const first = new Box<number>(40);
            const duplicate = new Box<number>(41);
            const text = new Box<string>("ready");
            const pair = new Pair<string, number>("answer", 42);
            const derived = new NumberBox(21);
            console.log(read(first));
            console.log(duplicate.replace(42));
            console.log(text.get());
            console.log(pair.left());
            console.log(pair.right());
            console.log(derived.double());
            console.log(derived.get());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_generic_classes"),
        "40\n42\nready\nanswer\n42\n42\n21\n"
    );
}

#[test]
fn compiles_constrained_and_defaulted_native_generic_classes() {
    let source = r#"
        class Box<T extends number = number> {
            constructor(public value: T) {}
            get(): T { return this.value; }
        }
        class Pair<T = string, U extends T = T> {
            constructor(public first: T, public second: U) {}
        }
        class DefaultBox extends Box {
            double(): number { return this.value * 2; }
        }
        function read(value: Box): number { return value.get(); }
        function main(): void {
            const inferredDefault = new Box(40);
            const explicit = new Box<number>(2);
            const pair = new Pair("left", "right");
            const derived = new DefaultBox(21);
            console.log(read(inferredDefault) + explicit.get());
            console.log(pair.first);
            console.log(pair.second);
            console.log(derived.double());
            console.log(derived.get());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_generic_class_defaults"),
        "42\nleft\nright\n42\n21\n"
    );
}

#[test]
fn infers_native_generic_classes_from_constructor_arguments() {
    let source = r#"
        class Box<T = string> {
            constructor(public value: T) {}
            get(): T { return this.value; }
        }
        class Pair<T, U> {
            constructor(public first: T, public second: U) {}
        }
        class Values<T> {
            constructor(public values: T[]) {}
            first(): T { return this.values[0]; }
        }
        class RecordBox<T> {
            constructor(public value: T) {}
        }
        function main(): void {
            const number = new Box(42);
            const text = new Box("ready");
            const pair = new Pair("answer", 42);
            const values = new Values([40, 42]);
            const record = new RecordBox({ count: 42, label: "items" });
            console.log(number.get());
            console.log(text.get());
            console.log(pair.first);
            console.log(pair.second);
            console.log(values.first());
            console.log(record.value.count);
            console.log(record.value.label);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_generic_class_inference"),
        "42\nready\nanswer\n42\n40\n42\nitems\n"
    );
}

#[test]
fn infers_native_generic_classes_through_scoped_bindings() {
    let source = r#"
        class Box<T> {
            constructor(public value: T) {}
            get(): T { return this.value; }
        }
        function show(input: number, label: string): void {
            const forwarded = input + 2;
            const record = { count: forwarded, label };
            const fromParameter = new Box(input);
            const fromInitializer = new Box(forwarded);
            const fromMember = new Box(record.label);
            console.log(fromParameter.get());
            console.log(fromInitializer.get());
            console.log(fromMember.get());
            {
                const forwarded = "shadowed";
                const fromShadow = new Box(forwarded);
                console.log(fromShadow.get());
            }
            const afterShadow = new Box(record.count + 0);
            console.log(afterShadow.get());
        }
        function main(): void { show(40, "ready"); }
    "#;
    assert_eq!(
        compile_and_run(source, "native_generic_class_binding_inference"),
        "40\n42\nready\nshadowed\n42\n"
    );
}

#[test]
fn compiles_nested_native_generic_class_specializations() {
    let source = r#"
        class Box<T> {
            constructor(public value: T) {}
            get(): T { return this.value; }
        }
        class Holder<T> {
            constructor(public value: T) {}
            get(): T { return this.value; }
        }
        class Pair<T, U> {
            constructor(public first: T, public second: U) {}
        }
        function read(value: Holder<Box<number>>): number {
            const inner = value.get();
            return inner.get();
        }
        function main(): void {
            const boxed = new Box(42);
            const explicit = new Holder<Box<number>>(boxed);
            const inferred = new Holder(boxed);
            const pair = new Pair<Holder<Box<number>>, Box<string>>(
                explicit,
                new Box("nested")
            );
            const pairFirst = pair.first;
            const pairSecond = pair.second;
            console.log(read(explicit));
            console.log(read(inferred));
            console.log(read(pairFirst));
            console.log(pairSecond.get());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_nested_generic_classes"),
        "42\n42\n42\nnested\n"
    );
}

#[test]
fn shares_native_generic_class_static_state_across_specializations() {
    let source = r#"
        class Box<T> {
            static count: number = 0;
            static label: string = "box";
            static #secret: number = 40;
            static { Box.count += 1; }
            constructor(public value: T) { Box.count += 1; }
            static current(): number { return Box.count; }
            static describe(): string { return Box.label; }
            static reveal(): number { return Box.#secret; }
        }
        class Counter<T> {
            static total: number = 1;
            constructor(public value: T) {}
        }
        class DerivedCounter<T> extends Counter<T> {
            constructor(value: T) { super(value); }
            static bump(): number {
                DerivedCounter.total += 1;
                return DerivedCounter.total;
            }
        }
        class Registry<T> {
            static next: number = 41;
            static value(): number { Registry.next += 1; return Registry.next; }
        }
        function main(): void {
            const number = new Box(40);
            const text = new Box("ready");
            console.log(number.value);
            console.log(text.value);
            console.log(Box.count);
            console.log(Box<number>.count);
            console.log(Box.current());
            console.log(Box<string>.current());
            console.log(Box.describe());
            console.log(Box.reveal());
            const derivedNumber = new DerivedCounter(1);
            const derivedText = new DerivedCounter("two");
            console.log(DerivedCounter.bump());
            console.log(Counter.total);
            console.log(DerivedCounter.total);
            console.log(derivedNumber.value);
            console.log(derivedText.value);
            console.log(Registry.value());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_generic_class_static_state"),
        "40\nready\n3\n3\n3\n3\nbox\n40\n2\n2\n2\n1\ntwo\n42\n"
    );
}

#[test]
fn infers_native_generic_classes_from_function_call_results() {
    let source = r#"
        class Box<T> {
            constructor(public value: T) {}
            get(): T { return this.value; }
        }
        function annotated(): number { return 40; }
        function forwarded() { return annotated(); }
        function branched(flag: boolean) { return flag ? forwarded() : 41; }
        function label() { return String(42); }
        function localInitializer() {
            const base = 40;
            const offset = 2;
            return base + offset;
        }
        function main(): void {
            const makeText = (value: number): string => "value=" + String(value);
            const factory = { positive: (value: number): boolean => value > 0 };
            const direct = new Box(annotated());
            const throughForward = new Box(forwarded());
            const throughBranch = new Box(branched(true));
            const text = new Box(label());
            const local = new Box(localInitializer());
            const arrow = new Box(makeText(42));
            const method = new Box(factory.positive(1));
            console.log(direct.get());
            console.log(throughForward.get());
            console.log(throughBranch.get());
            console.log(text.get());
            console.log(local.get());
            console.log(arrow.get());
            console.log(method.get());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_generic_class_call_inference"),
        "40\n40\n40\n42\n42\nvalue=42\ntrue\n"
    );
}

#[test]
fn compiles_explicit_native_generic_class_methods() {
    let source = r#"
        class Box<T> {
            constructor(public value: T) {}
            convert<U>(value: U): U { return value; }
            second<U, V = string>(first: U, second: V): V { return second; }
            numeric<U extends number>(value: U): U { return value; }
            fallback<U = string>(): U { return "fallback"; }
            first<U>(values: U[]): U { return values[0]; }
            field<U>(value: { item: U }): U { return value.item; }
            collect<U>(first: U, ...rest: U[]): U { return rest[0]; }
            keep<U>(ignored: U): T { return this.value; }
            choose<U>(first: U, second: U): U { return second; }
            viaThis(): string { return this.convert("this-call"); }
            forwardThis<U>(value: U): U { return this.convert(value); }
            withDefault<U = string>(value: number = 50): number { return value; }
            async convertAsync<U>(value: U): Promise<U> { return value; }
            static identity<U>(value: U): U { return value; }
            static async identityAsync<U>(value: U): Promise<U> { return value; }
            static label: string = "static-this-field";
            static initialized: string = this.identity<string>(this.label);
            static count: number = 1;
            static stored: number = 0;
            static get currentLabel(): string { return this.label; }
            static get score(): number { return this.stored; }
            static set score(value: number) { this.stored = value; }
            static { this.count += 2; this.score = 4; this.score++; }
            static makeLabel(): string { return this.label; }
            static viaStaticThis(): string { return this.identity(this.currentLabel); }
            static inferStaticMethodResult(): string { return this.identity(this.makeLabel()); }
            static forwardStaticThis<U>(value: U): U { return this.identity(value); }
            static mutateStaticThis(): number {
                const old = this.count++;
                this.score += 2;
                return old + this.count + this.score;
            }
        }
        class DerivedBox extends Box<number> {}
        class Holder {
            constructor(public box: Box<number>) {}
        }
        abstract class GenericBase {
            abstract transform<U>(value: U): U;
        }
        class GenericDerived extends GenericBase {
            transform<V>(value: V): V { return value; }
        }
        class GenericSuperBase {
            decorate<U>(value: U): U { return value; }
        }
        class GenericSuperDerived extends GenericSuperBase {
            forward<V>(value: V): V { return super.decorate(value); }
            fixed(): string { return super.decorate<string>("explicit-super"); }
        }
        class StaticGenericBase {
            static inheritedLabel: string = "inherited-static-inference";
            static inheritedIdentity<U>(value: U): U { return value; }
        }
        class StaticGenericDerived extends StaticGenericBase {
            static runInherited(): string {
                return this.inheritedIdentity(this.inheritedLabel);
            }
        }
        class StaticThisBase {
            static selected: string = "base-static-this";
            static readSelected(): string { return this.selected; }
            static selectMethod(): string { return "base-static-method"; }
            static dispatchMethod(): string { return this.selectMethod(); }
        }
        class StaticThisMiddle extends StaticThisBase {}
        class StaticThisDerived extends StaticThisMiddle {
            static selected: string = "derived-static-this";
            static selectMethod(): string { return "derived-static-method"; }
        }
        async function main(): Promise<void> {
            const box = new Box(40);
            const derived = new DerivedBox(40);
            const holder = new Holder(box);
            const tuple: [string] = ["tuple-spread"];
            const restTuple: [number, number] = [52, 53];
            const applyTuple: [number, number] = [56, 57];
            const staticTuple: [string] = ["static-apply"];
            console.log(box.convert<string>("converted"));
            console.log(box.convert<number>(42));
            console.log(box.convert("inferred"));
            console.log(box.second<number>(1, "defaulted"));
            console.log(box.numeric<number>(43));
            console.log(box.numeric(44));
            console.log(box.fallback());
            console.log(box.first([46, 47]));
            console.log(box.field({ item: "nested" }));
            console.log(box.collect(48, 49));
            console.log(box.withDefault());
            console.log(box.withDefault<boolean>());
            console.log(derived.convert("inherited"));
            console.log(await box.convertAsync("async-method"));
            console.log(Box.identity<boolean>(true));
            console.log(Box.identity<string>("static"));
            console.log(Box.identity(45));
            console.log(await Box.identityAsync(51));
            console.log(holder.box.convert("member-receiver"));
            console.log(box.convert(...tuple));
            console.log(box.collect(51, ...restTuple));
            console.log(new GenericDerived().transform("generic-override"));
            const genericSuper = new GenericSuperDerived();
            console.log(genericSuper.forward("inferred-super"));
            console.log(genericSuper.fixed());
            const rebound = box.keep<string>.bind(new Box(54));
            console.log(rebound("bound-generic-method"));
            const partial = box.choose<string>.bind(box, "bound-first");
            console.log(partial("partial-bind"));
            const asyncBound = box.convertAsync<string>.bind(box);
            console.log(await asyncBound("bound-async"));
            console.log(box.keep<boolean>.call(new Box(55), true));
            console.log(box.withDefault<number>.call(box));
            console.log(box.collect<number>.apply(box, applyTuple));
            console.log(box.withDefault<number>.bind(box)());
            console.log(box.collect<number>.bind(box, 58)(59));
            console.log(Box.identity<string>.call(
                (console.log("static-this"), box),
                (console.log("static-arg"), "static-call")
            ));
            console.log(Box.identity<string>.apply(box, staticTuple));
            const staticBound = Box.identity<string>.bind(box);
            console.log(staticBound("static-bound"));
            const staticComplete = Box.identity<number>.bind(box, 60);
            console.log(staticComplete());
            const staticAsyncBound = Box.identityAsync<string>.bind(box);
            console.log(await staticAsyncBound("static-async-bound"));
            console.log(box.viaThis());
            console.log(box.forwardThis("nested-this-call"));
            console.log(Box.viaStaticThis());
            console.log(Box.forwardStaticThis("nested-static-this-call"));
            console.log(Box.mutateStaticThis());
            console.log(Box.initialized);
            console.log(Box.inferStaticMethodResult());
            console.log(StaticGenericDerived.runInherited());
            console.log(StaticThisDerived.readSelected());
            console.log(StaticThisDerived.dispatchMethod());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_generic_class_methods"),
        "converted\n42\ninferred\ndefaulted\n43\n44\nfallback\n46\nnested\n49\n50\n50\ninherited\nasync-method\ntrue\nstatic\n45\n51\nmember-receiver\ntuple-spread\n52\ngeneric-override\ninferred-super\nexplicit-super\n54\npartial-bind\nbound-async\n55\n50\n57\n50\n59\nstatic-this\nstatic-arg\nstatic-call\nstatic-apply\nstatic-bound\n60\nstatic-async-bound\nthis-call\nnested-this-call\nstatic-this-field\nnested-static-this-call\n14\nstatic-this-field\nstatic-this-field\ninherited-static-inference\nderived-static-this\nderived-static-method\n"
    );
}

#[test]
fn infers_generic_methods_from_inherited_instance_members() {
    let source = r#"
        class SourceBase {
            value: string = "field";
            get current(): string { return "getter"; }
            read(): string { return "method"; }
            convert<T>(value: T): T { return value; }
            fromThisField(): string { return this.convert(this.value); }
            fromThisGetter(): string { return this.convert(this.current); }
            fromThisMethod(): string { return this.convert(this.read()); }
        }
        class SourceDerived extends SourceBase {}
        function main(): void {
            const source = new SourceDerived();
            console.log(source.convert(source.value));
            console.log(source.convert(source.current));
            console.log(source.convert(source.read()));
            console.log(source.fromThisField());
            console.log(source.fromThisGetter());
            console.log(source.fromThisMethod());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_method_instance_member_inference"),
        "field\ngetter\nmethod\nfield\ngetter\nmethod\n"
    );
}

#[test]
fn binds_native_methods_with_typed_tuple_spreads() {
    let source = r#"
        class Binder {
            join(left: string, right: string): string { return left + right; }
            static join(left: string, right: string): string { return left + right; }
        }
        function instanceTuple(): [string] {
            console.log("instance-tuple");
            return ["instance-"];
        }
        function staticTuple(): [string, string] {
            console.log("static-tuple");
            return ["static-", "bound"];
        }
        function main(): void {
            const binder = new Binder();
            const instanceBound = binder.join.bind(
                (console.log("instance-this"), binder),
                ...instanceTuple()
            );
            console.log(instanceBound("bound"));
            const staticBound = Binder.join.bind(
                (console.log("static-this"), binder),
                ...staticTuple()
            );
            console.log(staticBound());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_method_bind_tuple_spreads"),
        "instance-this\ninstance-tuple\ninstance-bound\nstatic-this\nstatic-tuple\nstatic-bound\n"
    );
}

#[test]
fn calls_extracted_this_independent_native_methods() {
    let source = r#"
        class Operations {
            pass(value: string): string { return value; }
            async passAsync(value: string): Promise<string> { return value; }
            static double(value: number): number { return value * 2; }
        }
        async function main(): Promise<void> {
            const operations = new Operations();
            const pass = operations.pass;
            const passAsync = operations.passAsync;
            const double = Operations.double;
            console.log(pass("instance-reference"));
            console.log(await passAsync("async-reference"));
            console.log(double(21));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "extracted_native_method_values"),
        "instance-reference\nasync-reference\n42\n"
    );
}

#[test]
fn calls_saved_this_dependent_methods_with_explicit_receivers() {
    let source = r#"
        class Box {
            constructor(public value: string) {}
            read(suffix: string): string { return this.value + suffix; }
            async readAsync(suffix: string): Promise<string> { return this.value + suffix; }
            maybe(useThis: boolean): string {
                if (useThis) return this.value;
                return this === undefined ? "undefined-this" : "wrong-this";
            }
            async maybeAsync(useThis: boolean): Promise<string> {
                if (useThis) return this.value;
                return this === undefined ? "undefined-async-this" : "wrong-async-this";
            }
            write(value: string): string { this.value = value; return value; }
            append(suffix: string): string { this.value += suffix; return this.value; }
            format(label: string = "default", ...parts: string[]): string {
                if (label === "use-this") return this.value;
                return label + ":" + parts.join("|");
            }
            async formatAsync(label: string = "async-default", ...parts: string[]): Promise<string> {
                if (label === "use-this") return this.value;
                return label + ":" + parts.join("|");
            }
            defaultFromThis(value: string = this.value): string { return value; }
            static staticValue: string = "static";
            static staticRead(suffix: string): string { return this.staticValue + suffix; }
            static async staticReadAsync(suffix: string): Promise<string> {
                return this.staticValue + suffix;
            }
            static staticMaybe(useThis: boolean): string {
                if (useThis) return this.staticValue;
                return this === undefined ? "undefined-static-this" : "wrong-static-this";
            }
            static inspectThis(): string {
                return typeof this + ":" + (this === undefined ? "undefined" : "bound");
            }
        }
        class Holder { constructor(public box: Box) {} }
        class DerivedBox extends Box {}
        function make(): Box {
            console.log("extract-receiver");
            return new Box("discarded");
        }
        function passString(callback: (value: string) => string): (value: string) => string {
            return callback;
        }
        function passAsyncString(callback: (value: string) => Promise<string>): (value: string) => Promise<string> {
            return callback;
        }
        async function main(): Promise<void> {
            const read = make().read;
            const extracted = new Box("ignored");
            const readAsync = extracted.readAsync;
            const maybe = extracted.maybe;
            const maybeAsync = extracted.maybeAsync;
            const write = extracted.write;
            const append = extracted.append;
            const format = extracted.format;
            const formatAsync = extracted.formatAsync;
            const defaultFromThis = extracted.defaultFromThis;
            const inheritedFormat = new DerivedBox("derived").format;
            const passedRead = passString(extracted.read);
            const passedStaticRead = passString(Box.staticRead);
            const passedReadAsync = passAsyncString(extracted.readAsync);
            const holderRead = new Holder(new Box("holder")).box.read;
            const alias = read;
            const args: [string] = ["?"];
            const boundArgs: [string] = ["!"];
            const bound = alias.bind(new Box("bound"), ...boundArgs);
            const asyncBound = readAsync.bind(new Box("async-bound"));
            const staticRead = Box.staticRead;
            const staticReadAsync = Box.staticReadAsync;
            const staticMaybe = Box.staticMaybe;
            const inspectThis = Box.inspectThis;
            const staticBound = staticRead.bind(
                (console.log("static-bind-this"), extracted),
                ...boundArgs
            );
            const boundaryBound = passedRead.bind(new Box("boundary-bound"), ...boundArgs);
            const boundaryAsyncBound = passedReadAsync.bind(new Box("boundary-async-bound"), ...boundArgs);
            const boundaryStaticBound = passedStaticRead.bind(
                (console.log("boundary-static-bind-this"), extracted), ...boundArgs
            );
            console.log(read.call(new Box("call"), "!"));
            console.log(alias.apply(new Box("apply"), args));
            console.log(await readAsync.call(new Box("async"), "!"));
            console.log(bound());
            console.log(await asyncBound("!"));
            console.log(staticRead.call(
                (console.log("static-call-this"), extracted),
                "!"
            ));
            console.log(await staticReadAsync.apply(extracted, args));
            console.log(staticBound());
            console.log(holderRead.call(new Box("chain"), "!"));
            let mutable = read;
            const ordinary = (value: string): string => value;
            mutable = ordinary;
            console.log(mutable("ordinary"));
            mutable = extracted.read;
            console.log(mutable.call(new Box("reassigned"), "!"));
            console.log(maybe(false));
            console.log(await maybeAsync(false));
            console.log(staticMaybe(false));
            console.log(Box.inspectThis());
            console.log(inspectThis());
            try { maybe(true); } catch (error) { console.log(error); }
            try { await maybeAsync(true); } catch (error) { console.log(error); }
            try { await readAsync("!"); } catch (error) { console.log(error); }
            try { staticMaybe(true); } catch (error) { console.log(error); }
            try { write((console.log("assignment-rhs"), "changed")); } catch (error) { console.log(error); }
            try { append((console.log("compound-rhs"), "!")); } catch (error) { console.log(error); }
            console.log(format());
            console.log(format(undefined, "a", "b"));
            console.log(format("plain", "x", "y"));
            console.log(await formatAsync());
            console.log(defaultFromThis("explicit"));
            console.log(inheritedFormat());
            console.log(passedRead.call(new Box("boundary-call"), "!"));
            console.log(passedRead.apply(new Box("boundary-apply"), args));
            console.log(passedStaticRead.call((console.log("boundary-static-this"), extracted), "!"));
            console.log(await passedReadAsync.call(new Box("boundary-async"), "!"));
            console.log(boundaryBound());
            console.log(boundaryBound.call(new Box("ignored-rebind")));
            console.log(await boundaryAsyncBound());
            console.log(boundaryStaticBound());
            try { defaultFromThis(); } catch (error) { console.log(error); }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "saved_unbound_native_method_call_apply"),
        "extract-receiver\nstatic-bind-this\nboundary-static-bind-this\ncall!\napply?\nasync!\nbound!\nasync-bound!\nstatic-call-this\nstatic!\nstatic?\nstatic!\nchain!\nordinary\nreassigned!\nundefined-this\nundefined-async-this\nundefined-static-this\nfunction:bound\nundefined:undefined\nCannot read properties of undefined (reading 'value')\nCannot read properties of undefined (reading 'value')\nCannot read properties of undefined (reading 'value')\nCannot read properties of undefined (reading 'staticValue')\nassignment-rhs\nCannot set properties of undefined (setting 'value')\ncompound-rhs\nCannot read properties of undefined (reading 'value')\ndefault:\ndefault:a|b\nplain:x|y\nasync-default:\nexplicit\ndefault:\nboundary-call!\nboundary-apply?\nboundary-static-this\nstatic!\nboundary-async!\nboundary-bound!\nboundary-bound!\nboundary-async-bound!\nstatic!\nCannot read properties of undefined (reading 'value')\n"
    );
}

#[test]
fn compiles_and_runs_native_class_instance_methods() {
    let source = r#"
        class Counter {
            value: number;
            constructor(value: number) { this.value = value; }
            add(delta: number): number {
                this.value += delta;
                return this.value;
            }
        }
        function main(): void {
            const counter = new Counter(40);
            console.log(counter.add(2));
            console.log(counter.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_class_instance_methods"),
        "42\n42\n"
    );
}

#[test]
fn compiles_and_runs_native_class_static_methods() {
    let source = r#"
        class MathBox {
            static add(left: number, right: number): number { return left + right; }
            static label(value: string): string { return value; }
        }
        function main(): void {
            console.log(MathBox.add(40, 2));
            console.log(MathBox.label("ready"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_class_static_methods"),
        "42\nready\n"
    );
}

#[test]
fn compiles_and_runs_native_class_getters() {
    let source = r#"
        class Box {
            value: number;
            constructor(value: number) { this.value = value; }
            get doubled(): number { return this.value * 2; }
            static get version(): string { return "v1"; }
        }
        function main(): void {
            const box = new Box(21);
            console.log(box.doubled);
            console.log(Box.version);
        }
    "#;
    assert_eq!(compile_and_run(source, "native_class_getters"), "42\nv1\n");
}

/// A getter read off a receiver that isn't a plain identifier -- `new
/// Box(21).doubled`, `factory().doubled` -- which the class-getter
/// dispatch originally didn't see through.
#[test]
fn compiles_and_runs_getters_on_a_non_identifier_receiver() {
    let source = r#"
        class Box {
            value: number;
            constructor(value: number) { this.value = value; }
            get doubled(): number { return this.value * 2; }
            rebuild(): Box { return new Box(this.value); }
        }
        function main(): void {
            console.log(new Box(21).doubled);
            console.log(new Box(20).rebuild().doubled);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "getters_on_non_identifier_receiver"),
        "42\n40\n"
    );
}

#[test]
fn static_accessor_prefix_returns_computed_value_before_setter_normalization() {
    let source = r#"
        let stored = 0;
        let reads = 0;
        let writes = 0;
        let order = "";
        function reset(value: number): void {
            stored = value;
            reads = 0;
            writes = 0;
            order = "";
        }
        class Counter {
            static get value(): number {
                reads++;
                order += "G";
                return stored;
            }
            static set value(next: number) {
                writes++;
                order += "S";
                stored = next * 10;
            }
            static incrementViaThis(): number { return ++this.value; }
            static decrementViaThis(): number { return --this.value; }
        }
        function main(): void {
            reset(4);
            const incremented: number = ++Counter.value;
            console.log(incremented, stored, reads, writes, order);
            reset(4);
            const decremented: number = --Counter.value;
            console.log(decremented, stored, reads, writes, order);
            reset(4);
            const oldIncrement: number = Counter.value++;
            console.log(oldIncrement, stored, reads, writes, order);
            reset(4);
            const oldDecrement: number = Counter.value--;
            console.log(oldDecrement, stored, reads, writes, order);
            reset(2);
            console.log(Counter.incrementViaThis(), stored, reads, writes, order);
            reset(2);
            console.log(Counter.decrementViaThis(), stored, reads, writes, order);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "static_accessor_prefix_result"),
        "5 50 1 1 GS\n3 30 1 1 GS\n4 50 1 1 GS\n4 30 1 1 GS\n3 30 1 1 GS\n1 10 1 1 GS\n"
    );
}

#[test]
fn compiles_and_runs_native_class_setters() {
    let source = r#"
        let version: number = 0;
        class Box {
            stored: number;
            constructor(value: number) { this.stored = value; }
            get value(): number { return this.stored; }
            set value(next: number) { this.stored = next; }
            static set current(next: number) { version = next; }
        }
        function main(): void {
            const box = new Box(1);
            const assigned = (box.value = 40);
            const selected = (Box.current = 2);
            console.log(assigned + selected);
            console.log(box.value + version);
        }
    "#;
    assert_eq!(compile_and_run(source, "native_class_setters"), "42\n42\n");
}

#[test]
fn compiles_and_runs_constructor_parameter_properties() {
    let source = r#"
        class Point {
            constructor(public x: number, readonly label: string) {}
            sum(y: number): number { return this.x + y; }
        }
        function main(): void {
            const point = new Point(40, "ready");
            console.log(point.sum(2));
            console.log(point.label);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "constructor_parameter_properties"),
        "42\nready\n"
    );
}

#[test]
fn compiles_and_runs_super_initialization_on_the_derived_instance() {
    let source = r#"
        class Derived extends Base {
            label: string = "ready";
            constructor(value: number) { super(value); }
            answer(): number { return this.value; }
        }
        class Base { constructor(public value: number) {} }
        function main(): void {
            const value = new Derived(42);
            console.log(value.answer());
            console.log(value.label);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "super_initializes_derived_instance"),
        "42\nready\n"
    );
}

#[test]
fn compiles_and_runs_super_method_calls() {
    let source = r#"
        class Base {
            constructor(public value: number) {}
            answer(delta: number): number { return this.value + delta; }
        }
        class Derived extends Base {
            constructor(value: number) { super(value); }
            answer(delta: number): number { return super.answer(delta) + 1; }
        }
        function main(): void { console.log(new Derived(40).answer(1)); }
    "#;
    assert_eq!(compile_and_run(source, "super_method_calls"), "42\n");
}

#[test]
fn compiles_and_runs_super_accessors() {
    let source = r#"
        class Base {
            stored: number;
            constructor(value: number) { this.stored = value; }
            get value(): number { return this.stored; }
            set value(next: number) { this.stored = next; }
        }
        class Derived extends Base {
            constructor(value: number) { super(value); }
            get value(): number { return super.value + 1; }
            set value(next: number) { super.value = next + 1; }
        }
        function main(): void {
            const value = new Derived(1);
            console.log(value.value = 20);
            console.log(value.value);
        }
    "#;
    assert_eq!(compile_and_run(source, "super_accessors"), "20\n22\n");
}

#[test]
fn compiles_and_runs_inherited_static_members() {
    let source = r#"
        let stored: number = 0;
        class Base {
            static add(left: number, right: number): number { return left + right; }
            static get current(): number { return stored; }
            static set current(next: number) { stored = next; }
        }
        class Derived extends Base {
            static add(left: number, right: number): number { return left + right + 1; }
        }
        function main(): void {
            console.log(Derived.current = 40);
            console.log(Derived.current);
            console.log(Derived.add(1, 1));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "inherited_static_members"),
        "40\n40\n3\n"
    );
}

#[test]
fn compiles_and_runs_static_super_members() {
    let source = r#"
        let stored: number = 0;
        class Base {
            static add(value: number): number { return value + 1; }
            static get current(): number { return stored; }
            static set current(next: number) { stored = next; }
        }
        class Derived extends Base {
            static add(value: number): number { return super.add(value) + 1; }
            static get current(): number { return super.current + 1; }
            static set current(next: number) { super.current = next + 1; }
        }
        function main(): void {
            console.log(Derived.current = 40);
            console.log(Derived.current);
            console.log(Derived.add(0));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "static_super_members"),
        "40\n42\n2\n"
    );
}

#[test]
fn compiles_and_runs_implicit_derived_constructor_forwarding() {
    let source = r#"
        class Leaf extends Middle {}
        class Middle extends Base {}
        class Base {
            constructor(public value: number, public label: string) {}
            answer(): number { return this.value; }
        }
        function main(): void {
            const value = new Leaf(42, "ready");
            console.log(value.answer());
            console.log(value.label);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "implicit_derived_constructor_forwarding"),
        "42\nready\n"
    );
}

#[test]
fn promise_constructors_preserve_union_values_and_discriminants() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        async function main(): Promise<void> {
            const promised = new Promise<Result>(resolve => {
                resolve({ kind: "number", value: 1 });
            });
            const inferred = await promised;
            print(inferred);
            const direct = await new Promise<Result>(resolve => {
                resolve({ kind: "text", value: "direct" });
            });
            print(direct);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_union_constructor"),
        "11\ndirect!\n"
    );
}

#[test]
fn promise_constructor_then_and_catch_chain_native_values() {
    let source = r#"
        async function main(): Promise<void> {
            const fulfilled: Promise<number> = new Promise<number>((resolve, reject) => {
                resolve(20);
            }).then(value => value + 1).then(value => value * 2);
            const recovered: Promise<number> = new Promise<number>((resolve, reject) => {
                reject("expected failure");
            }).catch(error => {
                console.log(error);
                return 42;
            });
            const callbackThrow: Promise<number> = new Promise<number>((resolve, reject) => {
                resolve(1);
            }).then(value => {
                throw "callback failure";
                return 0;
            }).catch(error => {
                console.log(error);
                return 7;
            });
            const executorThrow: Promise<number> = new Promise<number>((resolve, reject) => {
                throw "executor failure";
            }).catch(error => {
                console.log(error);
                return 8;
            });
            const inferredLocal: number = await new Promise((resolve, reject) => {
                const base = 20;
                const answer = base + 22;
                resolve(answer);
            });
            console.log(await fulfilled);
            console.log(await recovered);
            console.log(await callbackThrow);
            console.log(await executorThrow);
            console.log(inferredLocal);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_constructor_chains"),
        "expected failure\nexecutor failure\ncallback failure\n42\n42\n7\n8\n42\n"
    );
}

#[test]
fn promise_void_constructor_resolves_and_rejects() {
    let source = r#"
        async function main(): Promise<void> {
            await new Promise<void>((resolve, reject) => {
                console.log("executor");
                resolve();
            });
            try {
                await new Promise<void>((resolve, reject) => {
                    reject("void failure");
                });
            } catch (error) {
                console.log(error);
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_void_constructor"),
        "executor\nvoid failure\ndone\n"
    );
}

/// TypeScript's legacy decorator syntax (`@expr` on a class/property/
/// method) was previously rejected at the parser level entirely
/// (`thaw-parser`'s `TsSyntax { decorators: false, .. }`). Decorators are
/// now real function calls, evaluated in the same order real
/// `--experimentalDecorators` TS emits: each member's decorators (in
/// declaration order), then the class's own decorators, last. A bare
/// identifier (`@LogProp`) and a factory call (`@LogClass("greeter")`,
/// which must itself be invoked first, then its *result* called) both
/// work, dispatched through the ordinary native call path -- no dynamic
/// (`JsValue`) machinery is involved for a same-file/native decorator.
///
/// A class decorator receives a real, live "class token" `JsValue`
/// (`typeof target === "function"`); a property/method decorator
/// receives that token's own `.prototype` (`typeof target === "object"`,
/// and `target.constructor === <the class token>`), matching real JS's
/// own target shape for each decorator kind. Two decorators on the same
/// class see the *same* token by reference (`===`); a different class
/// gets its own distinct one -- this identity is what lets a real
/// `object.constructor`-keyed metadata store (class-validator's own,
/// see the pinned `class_validator_decorator_metadata_round_trips_
/// through_a_real_instance` integration test) find its own registration
/// again later.
#[test]
fn decorators_run_with_real_identity_in_declaration_order() {
    let source = r#"
        function LogClass(tag: string): (target: JsValue) => void {
            return function (target: JsValue) {
                console.log("class:", tag, typeof target);
            };
        }
        let lastPropTarget: JsValue = String;
        function LogProp(target: JsValue, key: string): void {
            lastPropTarget = target;
            console.log("prop:", typeof target, key);
        }
        function LogMethod(target: JsValue, key: string): void {
            console.log("method:", typeof target, key, target === lastPropTarget);
        }

        @LogClass("greeter")
        class Greeter {
            @LogProp
            name: string = "world";

            @LogMethod
            greet(): void {
                console.log("hello", this.name);
            }
        }

        function main(): void {
            const g = new Greeter();
            g.greet();
        }
    "#;
    assert_eq!(
        compile_and_run(source, "decorators_real_identity_order"),
        "prop: object name\nmethod: object greet true\nclass: greeter function\nhello world\n"
    );
}

/// The same class token identity `decorators_run_with_real_identity_in_
/// declaration_order` checks in isolation, but across *two different*
/// classes: each decorated class gets its own distinct token (never
/// confused with another class's), and the *same* class's property and
/// class decorators all agree on the one token that class owns.
#[test]
fn decorator_class_tokens_are_distinct_per_class() {
    let source = r#"
        let seen: JsValue[] = [];
        function Capture(target: JsValue): void {
            seen.push(target);
        }
        function CaptureProp(target: JsValue, key: string): void {
            seen.push(target.constructor);
        }

        @Capture
        class Foo {
            @CaptureProp
            x: number = 1;
        }

        @Capture
        class Bar {
            @CaptureProp
            y: number = 2;
        }

        function main(): void {
            console.log(seen[0] === seen[1]);
            console.log(seen[2] === seen[3]);
            console.log(seen[0] === seen[2]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "decorator_class_tokens_distinct"),
        "true\ntrue\nfalse\n"
    );
}

/// A class declaration with an instance method returning `any`/`Json`
/// (or a `Record<string, T>`/`Dictionary`) hard-errored at compile time
/// -- even with no instantiation or call anywhere in the program --
/// "unbound `this` cannot synthesize unreachable value of type Json".
/// `unreachable_value` (thaw-hir's `objects.rs`) builds a placeholder
/// value matching a method's declared return type for a context with no
/// real `this` receiver (e.g. type-checking the method's own signature
/// in isolation); it's never actually read at runtime (the whole branch
/// only exists to make a structurally-unreachable path type-check), but
/// had no case for `Json`/`Dictionary` among its `F64`/`Bool`/`Str`/
/// `Object`/`Array`/`Optional`/... cases. Fixed with an empty
/// `JsonObjectLit` placeholder for both, the same convention `Object`'s
/// own case already uses (`ObjectAlloc`).
#[test]
fn class_method_returning_dynamic_any_compiles() {
    let source = r#"
        class Box {
            value: any;
            constructor(v: any) {
                this.value = v;
            }
            get(): any {
                return this.value;
            }
        }
        function main(): void {
            const b = new Box({ a: 1 });
            console.log(JSON.stringify(b.get()));
            b.value = [1, 2, 3];
            console.log(JSON.stringify(b.value));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "class_method_returns_any"),
        "{\"a\":1}\n[1,2,3]\n"
    );
}

/// Sibling of `class_method_returning_dynamic_any_compiles` for
/// `Dictionary` (`Record<string, T>`) instead of bare `any`: a class
/// with both a `Record<string, number>`-typed field *and* a method
/// returning that same type. `unreachable_value`'s `HirType::
/// Dictionary(element)` case originally passed `ty` (already
/// `Dictionary(element)`) to `HirExpr::JsonObjectLit`, whose own type
/// inference *always* wraps its argument in another `Dictionary` --
/// producing a doubly-wrapped `Dictionary(Dictionary(element))`
/// placeholder instead of matching `ty`. That type mismatch, from a
/// placeholder that's never actually read, leaked into unrelated
/// class-member type inference and broke the constructor's own
/// `Record<string, number>` argument coercion elsewhere in the same
/// class ("dictionary value must be an object literal with F64
/// values") -- fixed by passing the *inner* `element` instead.
#[test]
fn class_method_returning_dictionary_compiles() {
    let source = r#"
        class Bag {
            data: Record<string, number>;
            constructor(d: Record<string, number>) {
                this.data = d;
            }
            get(): Record<string, number> {
                return this.data;
            }
        }
        function main(): void {
            const b = new Bag({ x: 5 });
            console.log(JSON.stringify(b.get()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "class_method_returns_dictionary"),
        "{\"x\":5}\n"
    );
}

/// A class method with no explicit return type annotation previously
/// always got `HirType::Void` baked permanently into its signature
/// (`lower_fn_return_type`'s own `None => HirType::Void` default,
/// `declarations.rs`), unlike a free function -- which already deferred
/// to `HirType::Dynamic` when unannotated, resolved later by inferring
/// the real type from the body (`module/pipeline.rs`'s own `func.
/// return_type.is_none() && !is_extern => HirType::Dynamic` case, with
/// nothing analogous for methods). Confirmed via a direct probe this
/// was a real, silent correctness bug, not just a compile error: a
/// method returning a number/string/array *compiled successfully* but
/// returned the wrong value at runtime (`0` for every case) since
/// nothing ever read the real return value out of a `Void`-typed
/// call; a method returning a plain object literal instead hard-erred
/// ("unsupported property access on a value of type Void"). Fixed by
/// giving methods the identical `Dynamic`-deferral free functions
/// already had, resolved from the lowered body the same way
/// (`declarations.rs`'s `method_ret`), including the separate
/// "unbound" (detached, `.bind()`-style) variant a `this`-using method
/// also gets.
#[test]
fn unannotated_class_methods_infer_their_return_type() {
    let source = r#"
        class Box {
            value: number;
            constructor(value: number) {
                this.value = value;
            }
            getValue() {
                return this.value * 2;
            }
            getInfo() {
                return { value: this.value, doubled: this.value * 2 };
            }
        }
        class Plain {
            makeNum() {
                return 42;
            }
            makeStr() {
                return "hi";
            }
            makeObj() {
                return { a: 1, b: "hi" };
            }
        }
        function main(): void {
            const b = new Box(21);
            console.log(b.getValue());
            const info = b.getInfo();
            console.log(info.value, info.doubled);
            const fn = b.getValue.bind(b);
            console.log(fn());

            const p = new Plain();
            console.log(p.makeNum());
            console.log(p.makeStr());
            const obj = p.makeObj();
            console.log(obj.a, obj.b);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "unannotated_class_methods_infer_return_type"),
        "42\n21 42\n42\n42\nhi\n1 hi\n"
    );
}

/// A caller processed *before* its callee's own signature has converged
/// used to hard-abort the whole compile: the fixed-point convergence
/// loop that resolves an unannotated function/method's return type
/// (`module/pipeline.rs`) tried each function/method every iteration and
/// propagated the first hard failure immediately via `?`, so a caller
/// whose own lowering hit a strict, up-front type check (`JSON.
/// stringify`'s own argument validation is the confirmed real trigger --
/// it hard-errors on seeing `HirType::Dynamic` instead of tolerating it)
/// aborted before the loop ever got a *second* iteration where the
/// callee's real type would have been patched in. Confirmed via a real
/// probe that this reproduces with two plain top-level functions with no
/// classes involved at all, and separately with a class method calling
/// another method the same way -- fixed by making each function/
/// method's lowering attempt *within* the trial loop tolerant of
/// failure (skip and retry next iteration) instead of instantly fatal,
/// since the loop's own output is discarded anyway except for patching
/// signatures; the real, error-propagating final lowering pass runs
/// unchanged after the loop converges.
#[test]
fn dynamic_return_type_convergence_tolerates_out_of_order_callers() {
    let source = r#"
        function main(): void {
            console.log(JSON.stringify(makeObj()));
        }
        function makeObj() {
            return { a: 1, b: "hi" };
        }
        class Box {
            value: number;
            constructor(value: number) {
                this.value = value;
            }
            getInfo() {
                return { value: this.value, doubled: this.value * 2 };
            }
        }
        function useBox(): void {
            const b = new Box(21);
            console.log(JSON.stringify(b.getInfo()));
        }
        useBox();
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_return_type_convergence_out_of_order"),
        "{\"value\":21,\"doubled\":42}\n{\"a\":1,\"b\":\"hi\"}\n"
    );
}

/// Sibling of the test above, but the out-of-order dependency is *within
/// one class*, not across free functions: `stepA` (declared first) calls
/// `this.stepB(x)` before `stepB`'s own unannotated return type has
/// converged. `lower_class_methods` used to lower every method of one
/// class as a single all-or-nothing `Result` (a plain `for` loop using
/// `?`), so `stepA`'s failure during the convergence loop's trial pass
/// aborted the *entire class* for that iteration -- including `stepB`,
/// declared later in the same loop and never even attempted, so it could
/// never converge either: a genuine deadlock, not just "needs more
/// iterations" (confirmed empirically: raising the loop's own iteration
/// bound alone did not fix this). Fixed by making each individual
/// method's lowering attempt tolerant of failure (skip just that one
/// method, keep whatever already succeeded) during the trial pass only --
/// the real, error-propagating final pass is unchanged, confirmed by
/// `class_method_chain_deadlock_still_reports_a_real_error` below still
/// failing correctly for a genuinely unresolvable case.
#[test]
fn class_method_chain_with_an_out_of_order_dependency_converges() {
    let source = r#"
        class Chain {
            step1(x: number) { return this.step2(x); }
            step2(x: number) { return this.step3(x); }
            step3(x: number) { return this.step4(x); }
            step4(x: number) { return this.step5(x); }
            step5(x: number) { return x + 1; }
        }
        function main(): void {
            const chain = new Chain();
            console.log(chain.step1(10));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "class_method_chain_out_of_order"),
        "11\n"
    );
}

/// A user class's own `[Symbol.iterator]()` method (unannotated, like
/// the idiomatic real-world pattern) now works with `for...of`, once its
/// return type actually converges (the fix above) -- previously hit
/// `` `for...of` currently requires a typed array `` even though the
/// identical pattern on a plain object literal already worked, since
/// `for...of`'s own dispatch synthesizes a call to the renamed
/// `__thaw_symbol_iterator` method and needs *that* call's inferred
/// type to be the `{next: () => {value, done}}` shape `iterator_object_
/// adapter` (`statements/lowering.rs`) recognizes -- previously always
/// `Dynamic` at the point `main()` itself was lowered, for exactly the
/// same out-of-order reason as the test above. Each `next()` call here
/// deliberately returns one uniform shape (`{value: number, done:
/// boolean}` in both branches); TS's own idiomatic discriminated-union
/// `IteratorResult<T>` shape (`{value: T, done: false} | {value:
/// undefined, done: true}`) is covered by
/// `custom_symbol_iterator_with_a_discriminated_iterator_result` below,
/// which `iterator_object_adapter` now accepts as a `Union` of shapes.
#[test]
fn custom_symbol_iterator_on_a_class_works_with_for_of() {
    let source = r#"
        class Range {
            start: number;
            end: number;
            constructor(start: number, end: number) {
                this.start = start;
                this.end = end;
            }
            [Symbol.iterator]() {
                let current = this.start;
                const end = this.end;
                return {
                    next() {
                        const value = current;
                        const done = current >= end;
                        current++;
                        return { value: value, done: done };
                    }
                };
            }
        }
        function main(): void {
            const r = new Range(1, 5);
            for (const x of r) {
                console.log(x);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "custom_symbol_iterator_class_for_of"),
        "1\n2\n3\n4\n"
    );
}

/// The literal idiomatic case the comment above used to defer: an
/// unannotated method returning TS's own discriminated-union
/// `IteratorResult<T>` shape (`{value: T, done: false}` on one branch,
/// `{value: undefined, done: true}` on the other) instead of one
/// uniform object shape -- now compiles and produces the right values
/// (return-type union inference, `infer_return_type` in `statements/
/// narrowing.rs`, plus the already-existing `lower_union_property_
/// read` for reading `.value`/`.done` back off the union). Named
/// `step` rather than `next` to go through an ordinary method call, so
/// the read-back is exercised directly rather than through the
/// iterator protocol; the equivalent real `next()` via `for...of` is
/// covered by `custom_symbol_iterator_with_a_discriminated_iterator_
/// result` below.
#[test]
fn method_returns_a_discriminated_iterator_result_union() {
    let source = r#"
        class Range {
            start: number;
            end: number;
            constructor(start: number, end: number) {
                this.start = start;
                this.end = end;
            }
            step() {
                let current = this.start;
                const end = this.end;
                if (current < end) {
                    this.start = current + 1;
                    return { value: current, done: false };
                }
                return { value: undefined, done: true };
            }
        }
        function main(): void {
            const r = new Range(1, 3);
            let result = r.step();
            while (!result.done) {
                console.log(result.value);
                result = r.step();
            }
            console.log("done", result.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "method_discriminated_iterator_result_union"),
        "1\n2\ndone undefined\n"
    );
}

/// The real `for...of` case for TS's idiomatic discriminated
/// `IteratorResult<T>`: an unannotated `[Symbol.iterator]` whose `next()`
/// returns `{value: T, done: false}` on one branch and `{value: undefined,
/// done: true}` on the other. Requires both the union return inference
/// (`infer_return_type`) and `iterator_object_adapter`'s own union
/// handling (`read_iterator_result_field`, `statements/lowering.rs`).
#[test]
fn custom_symbol_iterator_with_a_discriminated_iterator_result() {
    let source = r#"
        class Range {
            start: number;
            end: number;
            constructor(start: number, end: number) {
                this.start = start;
                this.end = end;
            }
            [Symbol.iterator]() {
                let current = this.start;
                const end = this.end;
                return {
                    next() {
                        if (current < end) {
                            const value = current;
                            current++;
                            return { value: value, done: false };
                        }
                        return { value: undefined, done: true };
                    }
                };
            }
        }
        function main(): void {
            const r = new Range(1, 5);
            for (const x of r) {
                console.log(x);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "custom_symbol_iterator_discriminated_result"),
        "1\n2\n3\n4\n"
    );
}

#[test]
fn non_arrow_function_expression_forwards_json_receiver() {
    let source = r#"
        function main(): void {
            const add: (delta: number) => number = function(this: Json, delta: number): number {
                return Number(this["base"]) + delta;
            };
            const holder: Json = { base: 7 };
            console.log(add.call(holder, 3));
            const other: Json = { base: 10 };
            const bound = add.bind(other);
            console.log(bound(2));
            const plain = function(delta: number): number { return delta + 1; };
            console.log(plain(4));
        }
    "#;
    assert_eq!(compile_and_run(source, "non_arrow_json_this"), "10\n12\n5\n");
}

#[test]
fn nested_non_arrow_receiver_does_not_capture_outer_this() {
    let source = r#"
        function main(): void {
            const outer = function(this: Json): number {
                const lexical = (): number => Number(this["base"]);
                const inner = function(this: Json): number { return Number(this["base"]); };
                const innerHolder: Json = { base: 20 };
                return inner.call(innerHolder) + lexical();
            };
            const outerHolder: Json = { base: 3 };
            console.log(outer.call(outerHolder));
        }
    "#;
    assert_eq!(compile_and_run(source, "nested_non_arrow_receivers"), "23\n");
}

#[test]
fn non_arrow_function_expression_generator_keeps_this_receiver() {
    let source = r#"
        function main(): void {
            const make = function*(this: Json, increment: number): Generator<number, void, undefined> {
                yield Number(this["base"]) + increment;
            };
            const holder: Json = { base: 7 };
            const iterator = make.call(holder, 3);
            console.log(iterator.next().value);
        }
    "#;
    assert_eq!(compile_and_run(source, "non_arrow_generator_this"), "10\n");
}

#[test]
fn non_arrow_function_expression_validates_native_class_receiver() {
    let source = r#"
        class Base { value: number; constructor(value: number) { this.value = value; } }
        class Derived extends Base { constructor(value: number) { super(value); } }
        class Other { value: number; constructor(value: number) { this.value = value; } }
        function main(): void {
            const read = function(this: Base): number { return this.value; };
            console.log(read.call(new Base(3)));
            console.log(read.call(new Derived(4)));
            try { read.call(new Other(5) as Base); } catch (error) { console.log("TypeError"); }
        }
    "#;
    assert_eq!(compile_and_run(source, "non_arrow_native_this"), "3\n4\nTypeError\n");
}

#[test]
fn generic_non_arrow_function_infers_this_for_call_apply_and_bind() {
    let source = r#"
        function main(): void {
            const choose = function<T>(this: T, value: T): T { return this; };
            const alias = choose;
            console.log(alias.call(7, 7));
            console.log(alias.apply(8, [8] as [number]));
            const bound = alias.bind(9);
            console.log(bound(9));
        }
    "#;
    assert_eq!(compile_and_run(source, "generic_non_arrow_this"), "7\n8\n9\n");
}

#[test]
fn generic_non_arrow_scoped_alias_and_empty_operations() {
    let source = r#"
        function main(): void {
            const choose = function<T>(this: T): T { return this; };
            {
                const choose = function<T>(this: T, value: T): T { return value; };
                console.log(choose.call(3, 4));
            }
            console.log(choose.call(5));
            console.log(typeof choose.call());
            console.log(typeof choose.apply());
            console.log(typeof choose.bind()());
            console.log(choose.apply(6));
            console.log(choose.apply(7, null));
            console.log(choose.apply(8, undefined));
            let effects = 0;
            function missing(): null { effects++; return null; }
            console.log(choose.apply(9, missing()));
            console.log(effects);
            const order: string[] = [];
            function receiver(): number { order.push("receiver"); return 10; }
            function tuple(): [] { order.push("tuple"); return []; }
            function extra(): number { order.push("extra"); return 1; }
            function absent(): null { order.push("null"); return null; }
            console.log(choose.apply(receiver(), tuple(), extra()));
            console.log(order.join(","));
            order.length = 0;
            console.log(choose.apply(receiver(), absent(), extra()));
            console.log(order.join(","));
        }
    "#;
    assert_eq!(compile_and_run(source, "generic_this_scoped_empty"),
        "4\n5\nundefined\nundefined\nundefined\n6\n7\n8\n9\n1\n10\nreceiver,tuple,extra\n10\nreceiver,null,extra\n");
}

#[test]
fn generic_non_arrow_optional_this_operations() {
    let source = r#"
        function main(): void {
            const choose = function<T>(this: T, value?: T): T { return this; };
            console.log(choose.call(7));
            console.log(choose.apply(8, null));
            console.log(choose.apply(9, undefined));
            const bound = choose.bind(10);
            console.log(bound());
            console.log(bound(10));
        }
    "#;
    assert_eq!(compile_and_run(source, "generic_this_optional"), "7\n8\n9\n10\n10\n");
}

#[test]
fn generic_non_arrow_rest_this_operations() {
    let source = r#"
        function main(): void {
            const count = function<T>(this: T, ...values: T[]): number {
                return values.length;
            };
            console.log(count.call(7, 7, 7));
            console.log(count.apply(8, [8, 8] as [number, number]));
            const bound = count.bind(9);
            console.log(bound(9, 9));
        const partial = count.bind(10, 10);
        console.log(partial(10));
        }
    "#;
    assert_eq!(compile_and_run(source, "generic_this_rest"), "2\n2\n2\n2\n");
}

#[test]
fn generic_non_arrow_apply_runtime_nullish_tuple() {
    let source = r#"
        function main(): void {
            const order: string[] = [];
            const choose = function<T>(this: T, value?: T): T {
                order.push("body");
                return this;
            };
            function receiver(): number { order.push("receiver"); return 7; }
            function maybe(mode: number): [number] | null | undefined {
                order.push("tuple");
                if (mode === 0) return null;
                if (mode === 1) return undefined;
                return [7] as [number];
            }
            function extra(): number { order.push("extra"); return 0; }
            function spreadExtra(): [number, number] { order.push("spread"); return [1, 2]; }
            console.log(choose.apply(receiver(), maybe(0), extra()));
            console.log(choose.apply(receiver(), maybe(1), extra()));
            console.log(choose.apply(receiver(), maybe(2), extra()));
            console.log(choose.apply(receiver(), maybe(0), ...spreadExtra()));
            console.log(order.join(","));
        }
    "#;
    assert_eq!(compile_and_run(source, "generic_this_nullish_tuple"),
        "7\n7\n7\n7\nreceiver,tuple,extra,body,receiver,tuple,extra,body,receiver,tuple,extra,body,receiver,tuple,spread,body\n");
}

#[test]
fn non_arrow_nullable_and_disjoint_union_receivers() {
    let source = r#"
        function main(): void {
            const optional = function(this: number | undefined): string { return typeof this; };
            const nullable = function(this: number | null): string { return typeof this; };
            const nullish = function(this: number | null | undefined): string { return typeof this; };
            const choice = function(this: number | string): string { return typeof this; };
            const jsonOptional = function(this: Json | undefined): string { return typeof this; };
            console.log(optional(), optional.call(5), optional.call(undefined));
            try { console.log(nullable()); } catch (error) { console.log("TypeError"); }
            console.log(nullable.call(null), nullable.call(6));
            console.log(nullish(), nullish.call(null), nullish.call(7));
            try { console.log(choice()); } catch (error) { console.log("TypeError"); }
            console.log(choice.call(8), choice.call("x"));
            console.log(jsonOptional(), jsonOptional.call({ value: 1 } as Json));
        }
    "#;
    assert_eq!(compile_and_run(source, "non_arrow_nullable_union_receiver"),
        "undefined number undefined\nTypeError\nobject number\nundefined object number\nTypeError\nnumber string\nundefined object\n");
}


#[test]
fn non_arrow_receiver_union_routes_json_and_number_by_producer_kind() {
    let source = r#"
        function main(): void {
            const kind = function(this: Json | number): string { return typeof this; };
            const holder: Json = { value: 7 };
            console.log(kind.call(3), kind.call(holder));
            const broad = function(this: Json | string): string { return typeof this; };
            console.log(broad.call("text"), broad.call(holder));
        }
    "#;
    assert_eq!(compile_and_run(source, "non_arrow_union_json_number"),
        "number object\nstring object\n");
}

#[test]
fn non_arrow_receiver_union_checks_each_native_class_identity() {
    let source = r#"
        class Base { value: number; constructor(value: number) { this.value = value; } }
        class Derived extends Base { constructor(value: number) { super(value); } }
        class Other { value: number; constructor(value: number) { this.value = value; } }
        class Stranger { value: number; constructor(value: number) { this.value = value; } }
        function main(): void {
            const pair = function(this: Base | Other): string { return typeof this; };
            console.log(pair.call(new Base(1)), pair.call(new Other(2)));
            try { pair.call(new Stranger(3) as Base); }
            catch (error) { console.log("TypeError"); }
            const inherited = function(this: Base | Derived): string { return typeof this; };
            console.log(inherited.call(new Base(4)), inherited.call(new Derived(5)));
        }
    "#;
    assert_eq!(compile_and_run(source, "non_arrow_union_classes"),
        "object object\nTypeError\nobject object\n");
}

#[test]
fn non_arrow_string_literal_this_uses_string_receiver_abi() {
    let source = r#"
        function main(): void {
            const echo = function(this: "open"): string { return this; };
            console.log(echo.call("open"));
            try { echo.call("closed" as "open"); }
            catch (error) { console.log("TypeError"); }
        }
    "#;
    assert_eq!(compile_and_run(source, "non_arrow_literal_this"), "open\nTypeError\n");
}
#[test]
fn non_arrow_same_tag_string_union_selects_literal_and_fallback() {
    let source = r#"
        type Choice<T> = "open" | T;
        type Plain = "open" | "closed";
        function main(): void {
            const exact = function(this: "open" | "closed"): string { return typeof this; };
            console.log(exact.call("open"), exact.call("closed"));
            try { exact.call("other" as "open"); }
            catch (error) { console.log("TypeError"); }
            const wide = function(this: string | "open"): string { return typeof this; };
            console.log(wide.call("open"), wide.call("other"));
            const json = function(this: Json | "open"): string { return typeof this; };
            console.log(json.call("open"), json.call("other"));
            const genericAlias = function(this: Choice<"closed">): string { return typeof this; };
            console.log(genericAlias.call("open"), genericAlias.call("closed"));
            const nestedAlias = function(this: Choice<Choice<"closed">>): string { return typeof this; };
            console.log(nestedAlias.call("open"), nestedAlias.call("closed"));
            const plainAlias = function(this: Plain): string { return typeof this; };
            console.log(plainAlias.call("open"), plainAlias.call("closed"));
        }
    "#;
    assert_eq!(compile_and_run(source, "non_arrow_string_union"),
        "string string\nTypeError\nstring string\nstring string\nstring string\nstring string\nstring string\n");
}

#[test]
fn non_arrow_nested_receiver_wrappers_use_leaf_envelope() {
    let source = r#"
        type Maybe<T> = T | undefined;
        class Carrier { value: number; constructor(value: number) { this.value = value; } }
        function main(): void {
            const branch = function(this: Maybe<number> | string): string { return typeof this; };
            console.log(branch(), branch.call(3), branch.call("x"));
            const overlap = function(this: (number | undefined) | number): string { return typeof this; };
            console.log(overlap(), overlap.call(5));
            const nullish = function(this: (number | null) | undefined): string { return typeof this; };
            console.log(nullish(), nullish.call(null), nullish.call(6));
            const native = function(this: (Carrier | undefined) | string): string { return typeof this; };
            console.log(native(), native.call(new Carrier(1)), native.call("x"));
        }
    "#;
    assert_eq!(compile_and_run(source, "non_arrow_nested_receivers"),
        "undefined number string\nundefined number\nundefined object number\nundefined object string\n");
}

#[test]
fn prefix_property_updates_bind_receiver_and_key_once() {
    let source = r#"
        let receiverCalls = 0;
        const left = { value: 4 };
        const right = { value: 40 };
        function pick(): { value: number } {
            receiverCalls++;
            return receiverCalls === 1 ? left : right;
        }
        let dictionaryCalls = 0;
        let keyCalls = 0;
        let order = "";
        const first: Record<string, number> = { value: 7 };
        const second: Record<string, number> = { value: 70 };
        function pickDictionary(): Record<string, number> {
            dictionaryCalls++;
            order += "R";
            return dictionaryCalls === 1 ? first : second;
        }
        function pickKey(): string {
            keyCalls++;
            order += "K";
            return "value";
        }
        let getterCalls = 0;
        let setterCalls = 0;
        let stored = 3;
        interface Box { value: number; }
        const box: Box = {
            get value(): number { getterCalls++; order += "G"; return stored; },
            set value(next: number) { setterCalls++; order += "S"; stored = next * 10; },
        };
        let boxCalls = 0;
        function pickBox(): Box { boxCalls++; order += "B"; return box; }
        function main(): void {
            console.log(++pick().value, receiverCalls, left.value, right.value);
            console.log(++pickDictionary()[pickKey()], dictionaryCalls, keyCalls, order, first.value, second.value);
            order = "";
            console.log(++pickBox().value, boxCalls, getterCalls, setterCalls, order, stored);
            console.log(--left["value"], left.value);
            console.log(left.value++, left.value);
            const frozen = Object.freeze({ value: 9 });
            try { ++frozen.value; console.log("unexpected"); }
            catch (error) { console.log(error instanceof TypeError, frozen.value); }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "prefix_property_target_once"),
        "5 1 5 40\n8 1 1 RK 8 70\n4 1 1 1 BGS 40\n4 4\n4 5\ntrue 9\n"
    );
}
