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
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_static_class_fields"),
        "42\n42\n43\nready\n"
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
        const before = 1, Base = class Base {
            constructor(public value: number = 40) {}
            add(delta: number = this.value): number { return this.value + delta; }
        }, after = 2;
        const Derived = class Derived extends Base {
            static label: string = "derived";
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
            console.log(value instanceof Base);
            console.log(Derived.label);
            console.log(anonymous.text);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_class_expressions"),
        "1\n2\n40\n80\ntrue\nderived\nready\n"
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

