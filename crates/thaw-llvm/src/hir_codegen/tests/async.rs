#[test]
fn calls_async_rest_function_values_across_typed_boundaries() {
    let source = r#"
        async function collect(
            prefix: string, ...values: string[]
        ): Promise<string> {
            await sleep(1);
            return prefix + values.join("|");
        }
        function passAsync(
            callback: (prefix: string, ...values: string[]) => Promise<string>
        ): (prefix: string, ...values: string[]) => Promise<string> {
            return callback;
        }
        async function main(): Promise<void> {
            const through = passAsync(collect);
            console.log(await through.call(null, "call:", "a", "b"));
            console.log(await through.bind(null, "bound:", "a")("b", "c"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_rest_function_boundary"),
        "call:a|b\nbound:a|b|c\n"
    );
}

#[test]
fn calls_async_default_function_values_across_typed_boundaries() {
    let source = r#"
        async function delayed(value: number = 20): Promise<number> {
            await sleep(1);
            return value + 22;
        }
        function passAsync(
            callback: (value?: number) => Promise<number>
        ): (value?: number) => Promise<number> {
            return callback;
        }
        async function main(): Promise<void> {
            const through = passAsync(delayed);
            const bound = through.bind(null);
            console.log(await through());
            console.log(await through(1));
            console.log(await through.call(null));
            console.log(await bound());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_default_function_boundary"),
        "42\n23\n42\n42\n"
    );
}

#[test]
fn immediately_invokes_bound_async_function_values() {
    let source = r#"
        async function addLater(left: number, right: number): Promise<number> {
            await sleep(1);
            return left + right;
        }
        function passAsync(
            callback: (left: number, right: number) => Promise<number>
        ): (left: number, right: number) => Promise<number> {
            return callback;
        }
        async function main(): Promise<void> {
            console.log(await passAsync(addLater).bind(null, 40)(2));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "immediate_async_function_bind"),
        "42\n"
    );
}

#[test]
fn frame_split_extracts_awaits_from_array_spreads_left_to_right() {
    let source = r#"
        function first(): number {
            console.log("first");
            return 1;
        }
        function last(): number {
            console.log("last");
            return 5;
        }
        async function part(): Promise<number[]> {
            await sleep(1);
            console.log("part");
            return [2, 3];
        }
        async function value(): Promise<number> {
            await sleep(1);
            console.log("value");
            return 4;
        }
        async function main(): Promise<void> {
            const values: number[] = [first(), ...(await part()), await value(), last()];
            console.log(values.length);
            console.log(values[3]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "await_array_spreads"),
        "first\npart\nvalue\nlast\n5\n4\n"
    );
}

#[test]
fn frame_split_preserves_object_spreads_and_overrides_across_await() {
    let source = r#"
        interface Config { x: number; y: number; z: number; }
        function field(label: string, value: number): number {
            console.log(label);
            return value;
        }
        async function spread(): Promise<{ x: number; y: number }> {
            console.log("spread");
            await sleep(1);
            return { x: 9, y: 2 };
        }
        async function delayedField(): Promise<number> {
            console.log("last");
            await sleep(1);
            return 4;
        }
        async function main(): Promise<void> {
            const config: Config = {
                x: field("first", 1),
                ...(await spread()),
                x: field("override", 3),
                z: await delayedField()
            };
            console.log(config.x);
            console.log(config.y);
            console.log(config.z);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_spread_order_across_await"),
        "first\nspread\noverride\nlast\n3\n2\n4\n"
    );
}

#[test]
fn compiles_bitwise_not_with_awaited_operand() {
    let source = r#"
        async function numberValue(): Promise<number> {
            await sleep(1);
            return 5;
        }
        async function main(): Promise<void> {
            console.log(~5);
            console.log(~(await numberValue()));
            console.log(~~5);
        }
    "#;
    assert_eq!(compile_and_run(source, "bitwise_not"), "-6\n-6\n5\n");
}

#[test]
fn compiles_same_type_loose_equality_with_awaits() {
    let source = r#"
        async function numberValue(): Promise<number> {
            await sleep(1);
            return 2;
        }
        async function main(): Promise<void> {
            console.log(1 == 1);
            console.log("a" != "b");
            console.log(true == false);
            console.log((await numberValue()) == 2);
            console.log((await numberValue()) != 3);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "loose_equality"),
        "true\ntrue\nfalse\ntrue\ntrue\n"
    );
}

#[test]
fn compiles_typeof_for_native_values_and_awaits() {
    let source = r#"
        async function numberValue(): Promise<number> {
            await sleep(1);
            console.log("evaluated");
            return 2;
        }
        function callback(value: number): number { return value; }
        async function main(): Promise<void> {
            console.log(typeof (await numberValue()));
            console.log(typeof "text");
            console.log(typeof true);
            console.log(typeof callback);
            console.log(typeof [1, 2]);
            console.log(typeof { value: 1 });
            console.log(typeof numberValue());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "typed_typeof"),
        "evaluated\nnumber\nstring\nboolean\nfunction\nobject\nobject\nobject\n"
    );
}

#[test]
fn compiles_generic_async_functions_and_callbacks() {
    let source = r#"
        async function delayed<T>(value: T): Promise<T> {
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            console.log(await delayed<number>(14));
            console.log(await delayed("async"));
            const delayedNumber: (value: number) => Promise<number> = delayed<number>;
            console.log(await delayedNumber(15));
            const chained: number = await new Promise<number>((resolve, reject) => {
                resolve(16);
            }).then(delayed);
            console.log(chained);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_async_functions"),
        "14\nasync\n15\n16\n"
    );
}

#[test]
fn compiles_tagged_heterogeneous_unions_across_function_and_async_boundaries() {
    let source = r#"
        function identity(value: string | number): string | number { return value; }
        function triple(value: string | number | boolean): string | number | boolean {
            return value;
        }
        function equality(value: string | number): void {
            console.log(value === "same");
            console.log("same" === value);
            console.log(value === 2);
            console.log(value !== 2);
        }
        function unionNaN(value: string | number): boolean { return value === NaN; }
        function unionEqual(
            left: string | number,
            right: string | number
        ): boolean { return left === right; }
        function unionNotEqual(
            left: string | number,
            right: string | number
        ): boolean { return left !== right; }
        function conditional(flag: boolean): string | number {
            return flag ? "chosen" : 17;
        }
        function kind(value: string | number): string { return typeof value; }
        function describe(value: string | number): string {
            if (typeof value === "string") {
                return value + "!";
            } else {
                return String(value + 1);
            }
        }
        function describeReverse(value: string | number): string {
            if ("number" !== typeof value) {
                return value + "?";
            }
            return String(value * 2);
        }
        function reassigned(): number {
            let value: string | number = "initial";
            value = 4;
            return value + 1;
        }
        function render(value: string | number | boolean): string {
            if (typeof value === "string") return value + "!";
            if (typeof value === "number") return String(value + 1);
            return value ? "true" : "false";
        }
        function renderNested(value: string | number | boolean): string {
            if (typeof value === "string") {
                return value + "?";
            } else if (typeof value === "number") {
                return String(value * 2);
            } else {
                return value ? "yes" : "no";
            }
        }
        function fieldKind(value: { data: string | number }): string {
            return typeof value.data;
        }
        async function delayed(value: string | number): Promise<string | number> {
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            console.log(kind(identity("hello")));
            console.log(kind(identity(42)));
            console.log(identity("direct"));
            console.log(identity(12));
            console.log(triple(true));
            equality("same");
            equality(2);
            console.log(unionNaN(NaN));
            console.log(unionEqual("same", "same"));
            console.log(unionEqual("same", "other"));
            console.log(unionEqual(4, 4));
            console.log(unionEqual(4, "4"));
            console.log(unionEqual(NaN, NaN));
            console.log(unionNotEqual(4, "4"));
            console.log(conditional(true));
            console.log(conditional(false));
            const conditionalLocal: string | number = false ? "unused" : 18;
            console.log(conditionalLocal);
            console.log(describe("hello"));
            console.log(describe(42));
            console.log(describeReverse("ok"));
            console.log(describeReverse(5));
            console.log(reassigned());
            console.log(render("three"));
            console.log(render(2));
            console.log(render(false));
            console.log(renderNested("nested"));
            console.log(renderNested(3));
            console.log(renderNested(true));
            console.log(fieldKind({ data: "field" }));
            console.log(fieldKind({ data: 7 }));
            console.log(kind(await delayed("later")));
            console.log(kind(await delayed(9)));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tagged_heterogeneous_unions"),
        "string\nnumber\ndirect\n12\ntrue\ntrue\ntrue\nfalse\ntrue\nfalse\nfalse\ntrue\nfalse\nfalse\ntrue\nfalse\ntrue\nfalse\nfalse\ntrue\nchosen\n17\n18\nhello!\n43\nok?\n10\n5\nthree!\n3\nfalse\nnested?\n6\nyes\nstring\nnumber\nstring\nnumber\n"
    );
}

#[test]
fn compiles_nested_destructuring_with_rest_and_awaited_sources() {
    let source = r#"
        async function source(): Promise<{
            x: number; label: string; nested: { flag: boolean }; extra: number
        }> {
            await sleep(1);
            console.log("source");
            return { x: 1, label: "ok", nested: { flag: true }, extra: 4 };
        }
        function tupleSource(): [number, string, { value: number }, number, number] {
            console.log("tuple");
            return [2, "skip", { value: 3 }, 4, 5];
        }
        async function main(): Promise<void> {
            const { x: renamed, nested: { flag }, ...rest } = await source();
            console.log(renamed);
            console.log(flag);
            console.log(rest.label);
            console.log(rest.extra);
            const [first, , pair, ...tail]:
                [number, string, { value: number }, number, number] = tupleSource();
            console.log(first);
            console.log(pair.value);
            console.log(tail[0]);
            console.log(tail[1]);
            const [head, ...numbers] = [6, 7, 8];
            console.log(head);
            console.log(numbers[0]);
            console.log(numbers[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nested_destructuring"),
        "source\n1\ntrue\nok\n4\ntuple\n2\n3\n4\n5\n6\n7\n8\n"
    );
}

#[test]
fn compiles_for_of_and_for_await_destructuring() {
    let source = r#"
        async function pair(value: number, label: string): Promise<[number, string]> {
            await sleep(1); return [value, label];
        }
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        type ResultFactory = () => Result[];
        type AsyncResultFactory = () => Promise<Result[]>;
        interface ResultHolder { results: Result[]; }
        interface NestedResultHolder { payload: ResultHolder; }
        type HolderFactory = () => ResultHolder;
        type AsyncHolderFactory = () => Promise<ResultHolder>;
        type NestedHolderFactory = () => NestedResultHolder;
        interface ResultService {
            load: ResultFactory;
            holder: HolderFactory;
            nested: { load: ResultFactory };
        }
        type ServiceFactory = () => ResultService;
        type AsyncServiceFactory = () => Promise<ResultService>;
        interface HigherOrderService {
            service: ServiceFactory;
            asyncService: AsyncServiceFactory;
        }
        type HigherOrderFactory = () => HigherOrderService;
        interface DerivedService extends CombinedService, HolderServiceBase {
            nestedHolder: ResultHolder;
        }
        interface CombinedService extends ArrayServiceBase, LoaderServiceBase {}
        interface ArrayServiceBase { inheritedResults: Result[]; }
        interface LoaderServiceBase { inheritedLoad: ResultFactory; }
        interface HolderServiceBase { inheritedHolder: HolderFactory; }
        interface GenericResultHolder<T> { results: T[]; }
        interface GenericResultBase<T> { inheritedResults: T[]; }
        interface GenericResultService<T> extends GenericResultBase<T> {
            load: () => T[];
            holder: () => GenericResultHolder<T>;
        }
        async function result(number: boolean): Promise<Result> {
            await sleep(1);
            if (number) return { kind: "number", value: 8 };
            return { kind: "text", value: "async" };
        }
        function producedResults(): Result[] {
            return [{ kind: "number", value: 9 }];
        }
        async function delayedResults(): Promise<Result[]> {
            await sleep(1);
            return [{ kind: "text", value: "returned" }];
        }
        function producedHolder(): ResultHolder {
            return { results: [{ kind: "number", value: 14 }] };
        }
        async function delayedHolder(): Promise<ResultHolder> {
            await sleep(1);
            return { results: [{ kind: "text", value: "async holder" }] };
        }
        function producedNestedHolder(): NestedResultHolder {
            return {
                payload: { results: [{ kind: "number", value: 16 }] }
            };
        }
        function consumeResults(results: Result[]): string {
            for (const { kind, value } of results) {
                if (kind === "number") return String(value + 4);
                return value + " parameter";
            }
            return "empty";
        }
        function consumeFactory(factory: ResultFactory): string {
            for (const item of factory()) {
                if (item.kind === "number") return String(item.value + 4);
                return item.value + " factory";
            }
            return "empty";
        }
        function consumeHolder(holder: ResultHolder): string {
            for (const { kind, value } of holder.results) {
                if (kind === "number") return String(value + 6);
                return value + " holder parameter";
            }
            return "empty";
        }
        function consumeHolderFactory(factory: HolderFactory): string {
            for (const item of factory().results) {
                if (item.kind === "number") return String(item.value + 7);
                return item.value + " holder factory";
            }
            return "empty";
        }
        function consumeNestedHolder(holder: NestedResultHolder): string {
            for (const { kind, value } of holder.payload.results) {
                if (kind === "number") return String(value + 8);
                return value + " nested holder";
            }
            return "empty";
        }
        function consumeDestructuredHolder(
            { results }: ResultHolder
        ): string {
            for (const item of results) {
                if (item.kind === "number") return String(item.value + 9);
                return item.value + " destructured parameter";
            }
            return "empty";
        }
        function consumeService(service: ResultService): string {
            for (const item of service.load()) {
                if (item.kind === "number") return String(item.value + 10);
                return item.value + " service parameter";
            }
            return "empty";
        }
        function producedService(): ResultService {
            return {
                load: producedResults,
                holder: producedHolder,
                nested: { load: producedResults }
            };
        }
        async function delayedService(): Promise<ResultService> {
            await sleep(1);
            return {
                load: producedResults,
                holder: producedHolder,
                nested: { load: producedResults }
            };
        }
        function producedHigherOrderService(): HigherOrderService {
            return {
                service: producedService,
                asyncService: delayedService
            };
        }
        function consumeServiceFactory(factory: ServiceFactory): string {
            for (const item of factory().load()) {
                if (item.kind === "number") return String(item.value + 11);
                return item.value + " service factory";
            }
            return "empty";
        }
        function consumeHigherOrderService(service: HigherOrderService): string {
            for (const item of service.service().load()) {
                if (item.kind === "number") return String(item.value + 210);
                return item.value + " higher service";
            }
            return "empty";
        }
        function consumeDerivedService(service: DerivedService): string {
            for (const item of service.inheritedResults) {
                if (item.kind === "number") return String(item.value + 12);
                return item.value + " inherited array";
            }
            return "empty";
        }
        function consumeGenericHolder(holder: GenericResultHolder<Result>): string {
            for (const item of holder.results) {
                if (item.kind === "number") return String(item.value + 200);
                return item.value + " generic holder";
            }
            return "empty";
        }
        function consumeGenericService(service: GenericResultService<Result>): string {
            for (const item of service.load()) {
                if (item.kind === "number") return String(item.value + 201);
                return item.value + " generic load";
            }
            return "empty";
        }
        async function main(): Promise<void> {
            const rows = [
                { x: 1, nested: { flag: true } },
                { x: 2, nested: { flag: false } },
                { x: 3, nested: { flag: true } }
            ];
            for (const { x, nested: { flag } } of rows) {
                if (x === 2) continue;
                console.log(x); console.log(flag);
            }
            let assigned = 0;
            for ({ x: assigned } of rows) {
                if (assigned === 2) break;
                console.log(assigned);
            }
            const pending: Promise<[number, string]>[] = [
                pair(4, "four"), pair(5, "five")
            ];
            for await (const [value, label] of pending) {
                console.log(value); console.log(label);
            }
            const results: Result[] = [
                { kind: "number", value: 6 },
                { kind: "text", value: "sync" }
            ];
            const reversedResults = results
                .slice(0, 2)
                .concat(results.slice(0, 0))
                .filter(() => true)
                .toReversed()
                .reverse();
            for (const item of reversedResults) {
                if (item.kind === "number") console.log(item.value + 150);
                else console.log(item.value + " array method");
            }
            const copiedResults = results.copyWithin(0, 0);
            for (const item of copiedResults) {
                if (item.kind === "number") console.log(item.value + 151);
                else console.log(item.value + " copied");
            }
            for (const { kind, value } of results) {
                if (kind === "number") console.log(value + 1);
                else console.log(value + "!");
            }
            const resultsAlias = results;
            for (const item of resultsAlias) {
                if (item.kind === "number") console.log(item.value + 2);
                else console.log(item.value + " item");
            }
            const holder: ResultHolder = {
                results: [{ kind: "number", value: 12 }]
            };
            const holderAlias = holder;
            for (const item of holderAlias.results) {
                if (item.kind === "number") console.log(item.value + 1);
                else console.log(item.value + " holder");
            }
            console.log(consumeHolder({
                results: [{ kind: "text", value: "nested" }]
            }));
            for (const { kind, value } of producedHolder().results) {
                if (kind === "number") console.log(value + 1);
                else console.log(value + " produced holder");
            }
            for (const item of (await delayedHolder()).results) {
                if (item.kind === "number") console.log(item.value + 1);
                else console.log(item.value + "!");
            }
            const holderFactory: HolderFactory = producedHolder;
            const holderFactoryAlias = holderFactory;
            console.log(consumeHolderFactory(holderFactoryAlias));
            const asyncHolderFactory: AsyncHolderFactory = delayedHolder;
            for (const { kind, value } of (await asyncHolderFactory()).results) {
                if (kind === "number") console.log(value + 1);
                else console.log(value + " function value");
            }
            const nestedHolder: NestedResultHolder = {
                payload: { results: [{ kind: "text", value: "deep" }] }
            };
            const nestedHolderAlias = nestedHolder;
            console.log(consumeNestedHolder(nestedHolderAlias));
            const nestedFactory: NestedHolderFactory = producedNestedHolder;
            for (const item of nestedFactory().payload.results) {
                if (item.kind === "number") console.log(item.value + 1);
                else console.log(item.value + " nested factory");
            }
            const inferredHolder = { results: producedResults() };
            const payload = inferredHolder;
            const inferredNestedHolder = { payload };
            const spreadNestedHolder = { ...inferredNestedHolder };
            for (const item of spreadNestedHolder.payload.results) {
                if (item.kind === "number") console.log(item.value + 30);
                else console.log(item.value + " inferred holder");
            }
            const { results: extractedResults } = holder;
            for (const item of extractedResults) {
                if (item.kind === "number") console.log(item.value + 40);
                else console.log(item.value + " extracted");
            }
            const { payload: { results: deepResults } } = nestedHolder;
            for (const { kind, value } of deepResults) {
                if (kind === "number") console.log(value + 1);
                else console.log(value + " destructured");
            }
            console.log(consumeDestructuredHolder({
                results: [{ kind: "text", value: "parameter" }]
            }));
            const { ...holderRest } = holder;
            for (const { kind, value } of holderRest.results) {
                if (kind === "number") console.log(value + 50);
                else console.log(value + " rest");
            }
            const { ...nestedHolderRest } = nestedHolder;
            for (const item of nestedHolderRest.payload.results) {
                if (item.kind === "number") console.log(item.value + 1);
                else console.log(item.value + " nested rest");
            }
            const service: ResultService = {
                load: producedResults,
                holder: producedHolder,
                nested: { load: producedResults }
            };
            const serviceAlias = service;
            const optionalService: ResultService = service;
            for (const item of optionalService?.load()) {
                if (item.kind === "number") console.log(item.value + 160);
                else console.log(item.value + " optional service");
            }
            const optionalFactory: ResultFactory = producedResults;
            for (const item of optionalFactory?.()) {
                if (item.kind === "number") console.log(item.value + 161);
                else console.log(item.value + " optional factory");
            }
            const optionalLoad: ResultFactory = optionalService?.load;
            for (const item of optionalLoad()) {
                if (item.kind === "number") console.log(item.value + 162);
                else console.log(item.value + " optional member");
            }
            const optionalHolder: HolderFactory = optionalService?.holder;
            for (const item of optionalHolder().results) {
                if (item.kind === "number") console.log(item.value + 163);
                else console.log(item.value + " optional holder");
            }
            for (const item of serviceAlias.load()) {
                if (item.kind === "number") console.log(item.value + 60);
                else console.log(item.value + " service");
            }
            for (const { kind, value } of serviceAlias.holder().results) {
                if (kind === "number") console.log(value + 1);
                else console.log(value + " service holder");
            }
            for (const item of serviceAlias.nested.load()) {
                if (item.kind === "number") console.log(item.value + 70);
                else console.log(item.value + " nested service");
            }
            console.log(consumeService(serviceAlias));
            const inferredService = {
                load: producedResults,
                nested: { holder: producedHolder }
            };
            const spreadService = { ...inferredService };
            for (const item of spreadService.load()) {
                if (item.kind === "number") console.log(item.value + 80);
                else console.log(item.value + " inferred service");
            }
            for (const item of spreadService.nested.holder().results) {
                if (item.kind === "number") console.log(item.value + 2);
                else console.log(item.value + " inferred holder service");
            }
            for (const item of producedService().load()) {
                if (item.kind === "number") console.log(item.value + 90);
                else console.log(item.value + " returned service");
            }
            for (const item of (await delayedService()).holder().results) {
                if (item.kind === "number") console.log(item.value + 3);
                else console.log(item.value + " async service");
            }
            const serviceFactory: ServiceFactory = producedService;
            const serviceFactoryAlias = serviceFactory;
            console.log(consumeServiceFactory(serviceFactoryAlias));
            const asyncServiceFactory: AsyncServiceFactory = delayedService;
            for (const item of (await asyncServiceFactory()).nested.load()) {
                if (item.kind === "number") console.log(item.value + 100);
                else console.log(item.value + " async service factory");
            }
            const higherOrderService: HigherOrderService = producedHigherOrderService();
            console.log(consumeHigherOrderService(higherOrderService));
            for (const item of (await higherOrderService.asyncService()).nested.load()) {
                if (item.kind === "number") console.log(item.value + 211);
                else console.log(item.value + " async higher service");
            }
            const higherOrderFactory: HigherOrderFactory = producedHigherOrderService;
            for (const item of higherOrderFactory().service().holder().results) {
                if (item.kind === "number") console.log(item.value + 212);
                else console.log(item.value + " higher factory");
            }
            const {
                load: extractedLoad,
                holder: extractedServiceHolder,
                nested: { load: extractedNestedLoad }
            } = service;
            const { holder: omittedHolder, ...serviceRest } = service;
            if (omittedHolder().results.length === 0) console.log("empty holder");
            for (const item of extractedLoad()) {
                if (item.kind === "number") console.log(item.value + 110);
                else console.log(item.value + " extracted service");
            }
            for (const item of extractedServiceHolder().results) {
                if (item.kind === "number") console.log(item.value + 4);
                else console.log(item.value + " extracted holder");
            }
            for (const item of extractedNestedLoad()) {
                if (item.kind === "number") console.log(item.value + 120);
                else console.log(item.value + " extracted nested service");
            }
            for (const item of serviceRest.load()) {
                if (item.kind === "number") console.log(item.value + 130);
                else console.log(item.value + " service rest");
            }
            const derivedService: DerivedService = {
                nestedHolder: {
                    results: [{ kind: "text", value: "derived nested" }]
                },
                inheritedHolder: producedHolder,
                inheritedLoad: producedResults,
                inheritedResults: [{ kind: "number", value: 10 }]
            };
            console.log(consumeDerivedService(derivedService));
            for (const item of derivedService.inheritedLoad()) {
                if (item.kind === "number") console.log(item.value + 140);
                else console.log(item.value + " inherited load");
            }
            for (const item of derivedService.inheritedHolder().results) {
                if (item.kind === "number") console.log(item.value + 5);
                else console.log(item.value + " inherited holder");
            }
            for (const item of derivedService.nestedHolder.results) {
                if (item.kind === "number") console.log(item.value + 1);
                else console.log(item.value + "!");
            }
            const genericService: GenericResultService<Result> = {
                inheritedResults: [{ kind: "text", value: "generic" }],
                load: producedResults,
                holder: producedHolder
            };
            console.log(consumeGenericHolder({
                results: [{ kind: "number", value: 18 }]
            }));
            console.log(consumeGenericService(genericService));
            for (const item of genericService.inheritedResults) {
                if (item.kind === "number") console.log(item.value + 1);
                else console.log(item.value + " inherited");
            }
            for (const item of genericService.holder().results) {
                if (item.kind === "number") console.log(item.value + 202);
                else console.log(item.value + " generic service holder");
            }
            let assignedKind: string = "text";
            let assignedValue: number | string = "initial";
            for ({ kind: assignedKind, value: assignedValue } of results) {
                if (assignedKind === "number") console.log(assignedValue + 10);
                else console.log(assignedValue + " assigned");
            }
            let ordinaryKind: string = "text";
            let ordinaryValue: number | string = "initial";
            ({ kind: ordinaryKind, value: ordinaryValue } = results[0]);
            if (ordinaryKind === "number") console.log(ordinaryValue + 20);
            else console.log(ordinaryValue + " ordinary");
            ({ kind: ordinaryKind, value: ordinaryValue } = results[1]);
            if (ordinaryKind === "number") console.log(ordinaryValue + 20);
            else console.log(ordinaryValue + " ordinary");
            console.log(consumeResults([{ kind: "text", value: "loop" }]));
            for (const item of producedResults()) {
                if (item.kind === "number") console.log(item.value + 1);
                else console.log(item.value + "!");
            }
            const producedAlias = producedResults();
            for (const { kind, value } of producedAlias) {
                if (kind === "number") console.log(value + 2);
                else console.log(value + "!");
            }
            for (const { kind, value } of await delayedResults()) {
                if (kind === "number") console.log(value + 3);
                else console.log(value + "!");
            }
            const factory: ResultFactory = producedResults;
            console.log(consumeFactory(factory));
            const inlineFactory = (): Result[] => [
                { kind: "text", value: "inline" }
            ];
            const factoryAlias = inlineFactory;
            for (const { kind, value } of factoryAlias()) {
                if (kind === "number") console.log(value + 5);
                else console.log(value + "!");
            }
            const asyncFactory: AsyncResultFactory = delayedResults;
            for (const item of await asyncFactory()) {
                if (item.kind === "number") console.log(item.value + 1);
                else console.log(item.value + "!");
            }
            const pendingResults: Promise<Result>[] = [result(true), result(false)];
            for await (const { kind, value } of pendingResults) {
                if (kind === "number") console.log(value + 3);
                else console.log(value + "!");
            }
            const assignedPending: Promise<Result>[] = [result(false)];
            for await ({ kind: assignedKind, value: assignedValue } of assignedPending) {
                if (assignedKind === "number") console.log(assignedValue + 3);
                else console.log(assignedValue + " assigned async");
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "for_of_destructuring"),
        "1\ntrue\n3\ntrue\n1\n4\nfour\n5\nfive\n156\nsync array method\n157\nsync copied\n7\nsync!\n8\nsync item\n13\nnested holder parameter\n15\nasync holder!\n21\nasync holder function value\ndeep nested holder\n17\n39\n52\ndeep destructured\nparameter destructured parameter\n62\ndeep nested rest\n169\n170\n171\n177\n69\n15\n79\n19\n89\n16\n99\n17\n20\n109\n219\n220\n226\n119\n18\n129\n139\n22\n149\n19\nderived nested!\n218\n210\ngeneric inherited\n216\n16\nsync assigned\n26\nsync ordinary\nloop parameter\n10\n11\nreturned!\n13\ninline!\nreturned!\n11\nasync!\nasync assigned async\n"
    );
}

#[test]
fn compiles_function_arrow_and_promise_callback_parameter_destructuring() {
    let source = r#"
        function describe(
            { x, nested: { flag }, ...rest }:
                { x: number; nested: { flag: boolean }; label: string; extra: number },
            [first, ...tail]: [number, number, number]
        ): number {
            console.log(x); console.log(flag);
            console.log(rest.label); console.log(rest.extra);
            console.log(first); console.log(tail[0]); console.log(tail[1]);
            return x + first;
        }
        async function objectValue(): Promise<{ value: number; label: string }> {
            await sleep(1); return { value: 8, label: "eight" };
        }
        function arrowValue(): number {
            const pick = ({ value }: { value: number }): number => value + 1;
            return pick({ value: 6 });
        }
        async function main(): Promise<void> {
            console.log(describe(
                { x: 1, nested: { flag: true }, label: "ok", extra: 4 },
                [2, 3, 5]
            ));
            console.log(arrowValue());
            const chained: number = await objectValue().then(({ value }) => value + 2);
            console.log(chained);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "parameter_destructuring"),
        "1\ntrue\nok\n4\n2\n3\n5\n3\n7\n10\n"
    );
}

#[test]
fn compiles_sequence_expressions_with_await_and_rejection() {
    let source = r#"
        function effect(value: number): number {
            console.log(value); return value;
        }
        async function asyncEffect(value: number, fail: boolean): Promise<number> {
            await sleep(1);
            console.log(value);
            if (fail) throw "sequence failed";
            return value;
        }
        async function main(): Promise<void> {
            const result = (effect(1), await asyncEffect(2, false), effect(3), 4);
            console.log(result);
            try {
                const skipped = (effect(5), await asyncEffect(6, true), effect(7), 8);
                console.log(skipped);
            } catch (error) {
                console.log(error);
            }
            (effect(9), console.log("done"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "sequence_expressions"),
        "1\n2\n3\n4\n5\n6\nsequence failed\n9\ndone\n"
    );
}

#[test]
fn compiles_void_expressions_with_await_and_rejection() {
    let source = r#"
        function effect(): number {
            console.log("sync");
            return 1;
        }
        async function asyncEffect(fail: boolean): Promise<number> {
            await sleep(1);
            if (fail) throw "failed";
            console.log("async");
            return 2;
        }
        async function main(): Promise<void> {
            void effect();
            void (await asyncEffect(false));
            try {
                void (await asyncEffect(true));
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "void_expressions"),
        "sync\nasync\nfailed\n"
    );
}

#[test]
fn logical_operators_short_circuit_sync_and_awaited_operands() {
    let source = r#"
        function flag(label: string, value: boolean): boolean {
            console.log(label);
            return value;
        }
        async function asyncFlag(label: string, value: boolean): Promise<boolean> {
            await sleep(1);
            console.log(label);
            return value;
        }
        async function main(): Promise<void> {
            console.log(false && flag("wrong-sync-and", true));
            console.log(true || flag("wrong-sync-or", false));
            console.log(true && flag("sync-and", true));
            console.log(false || flag("sync-or", true));
            console.log(false && (await asyncFlag("wrong-async-and", true)));
            console.log(true || (await asyncFlag("wrong-async-or", false)));
            console.log(true && (await asyncFlag("async-and", true)));
            console.log(false || (await asyncFlag("async-or", true)));
            try {
                console.log(true && (await new Promise<boolean>((resolve, reject) => {
                    reject("logical rejection");
                })));
            } catch (error) {
                console.log(error);
            }
            console.log(false && (await new Promise<boolean>((resolve, reject) => {
                reject("skipped rejection");
            })));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "logical_short_circuit"),
        "false\ntrue\nsync-and\ntrue\nsync-or\ntrue\nfalse\ntrue\nasync-and\ntrue\nasync-or\ntrue\nlogical rejection\nfalse\n"
    );
}

#[test]
fn compiles_string_template_literals_with_ordered_and_awaited_interpolation() {
    let source = r#"
        function word(label: string, value: string): string {
            console.log(label);
            return value;
        }
        async function delayed(value: string): Promise<string> {
            await sleep(1);
            console.log("awaited");
            return value;
        }
        async function main(): Promise<void> {
            console.log(`plain`);
            console.log(`A:${word("first", "x")}:${word("second", "y")}:Z`);
            const enabled: boolean = true;
            console.log(`enabled=${enabled}, disabled=${false}`);
            console.log(String(enabled));
            console.log(`numbers=${0},${-0},${1.5},${1000000000000000000000},${0.0000001}`);
            console.log(String(0 / 0));
            console.log(`before:${await delayed("done")}:after`);
            console.log(``);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_templates"),
        "plain\nfirst\nsecond\nA:x:y:Z\nenabled=true, disabled=false\ntrue\nnumbers=0,0,1.5,1e+21,1e-7\nNaN\nawaited\nbefore:done:after\n\n"
    );
}

#[test]
fn compiles_primitive_string_addition_with_ordered_awaits() {
    let source = r#"
        function text(label: string, value: string): string {
            console.log(label);
            return value;
        }
        function number(label: string, value: number): number {
            console.log(label);
            return value;
        }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            console.log("awaited-number");
            return value;
        }
        async function main(): Promise<void> {
            console.log(text("left", "value=") + number("right", 42));
            console.log(7 + " items");
            console.log("enabled=" + true);
            console.log(false + " flag");
            console.log("large=" + 1000000000000000000000);
            console.log("async=" + (await delayed(9)));
            let accumulated: string = "total=";
            accumulated += number("compound", 12);
            accumulated += true;
            console.log(accumulated);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "primitive_string_addition"),
        "left\nright\nvalue=42\n7 items\nenabled=true\nfalse flag\nlarge=1e+21\nawaited-number\nasync=9\ncompound\ntotal=12true\n"
    );
}

#[test]
fn compiles_native_number_conversion_and_awaited_strings() {
    let source = r#"
        async function delayed(value: string): Promise<string> {
            await sleep(1);
            console.log("awaited-number-input");
            return value;
        }
        async function main(): Promise<void> {
            console.log(Number(true));
            console.log(Number(false));
            console.log(Number(7));
            console.log(Number(" 42.5 "));
            console.log(Number("0xff"));
            console.log(Number("0o10"));
            console.log(Number("0b101"));
            console.log(Number(""));
            console.log(Number("bad") === Number("bad"));
            console.log(Number(await delayed("12")));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_number_conversion"),
        "1\n0\n7\n42.5\n255\n8\n5\n0\nfalse\nawaited-number-input\n12\n"
    );
}

#[test]
fn compiles_async_arrows() {
    let source = r#"
        interface Worker { run(): Promise<number>; }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }
        async function pause(): Promise<void> {
            await sleep(1);
        }
        async function main(): Promise<void> {
            const offset: number = 2;
            const double: (value: number) => Promise<number> =
                async value => value * 2;
            console.log(await double(21));
            const choose: (enabled: boolean) => Promise<number> = async enabled => {
                if (enabled) return 40 + offset;
                return 0;
            };
            console.log(await choose(true));
            const finish: () => Promise<void> = async () => {
                console.log("done");
            };
            await finish();
            const worker: Worker = {
                run: async (): Promise<number> => 40 + offset,
            };
            console.log(await worker.run());
            const adopted: (value: number) => Promise<number> =
                async value => await delayed(value);
            console.log(await adopted(42));
            const adoptedBlock: (enabled: boolean) => Promise<number> = async enabled => {
                if (enabled) return await delayed(40 + offset);
                return await delayed(0);
            };
            console.log(await adoptedBlock(true));
            const suspended: (value: number) => Promise<number> = async value => {
                const loaded: number = await delayed(value);
                await sleep(1);
                return loaded + offset;
            };
            console.log(await suspended(40));
            const suspendedExpression: (value: number) => Promise<number> =
                async value => (await delayed(value)) + offset;
            console.log(await suspendedExpression(40));
            const suspendedVoid: () => Promise<void> = async () => {
                return await pause();
            };
            await suspendedVoid();
            console.log("void");
            const catches: () => Promise<string> = async () => {
                let message: string = "missed";
                try {
                    await new Promise<number>((resolve, reject) => reject("boom"));
                } catch (error) {
                    message = "caught " + error;
                }
                return message;
            };
            console.log(await catches());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "expression_bodied_async_arrow"),
        "42\n42\ndone\n42\n42\n42\n42\n42\nvoid\ncaught boom\n"
    );
}

#[test]
fn compiles_readonly_and_awaited_utility_types() {
    let source = r#"
        type Frozen = Readonly<{ value: number; label: string }>;
        type FrozenBox<T> = Readonly<{ value: T }>;
        type Loaded = Awaited<Promise<number>>;
        type DeepLoaded = Awaited<Promise<Promise<number>>>;
        type LoadedValue<T> = Awaited<Promise<T>>;
        type Present = NonNullable<number | null | undefined>;
        type PresentValue<T> = NonNullable<T>;
        function main(): void {
            const frozen: Frozen = { value: 40, label: "ready" };
            const box: FrozenBox<string> = { value: frozen.label };
            const loaded: Loaded = frozen.value + 2;
            const generic: LoadedValue<number> = loaded;
            const deep: DeepLoaded = generic;
            const present: Present = deep;
            const genericPresent: PresentValue<number | null> = present;
            console.log(box.value);
            console.log(genericPresent);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "readonly_and_awaited_utility_types"),
        "ready\n42\n"
    );
}

#[test]
fn preserves_union_discriminants_for_inferred_arrays_and_promise_all() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        async function number(): Promise<Result> {
            return { kind: "number", value: 1 };
        }
        async function text(): Promise<Result> {
            return { kind: "text", value: "async" };
        }
        function print(values: Result[]): void {
            for (const item of values) {
                if (item.kind === "number") console.log(item.value + 10);
                else console.log(item.value + "!");
            }
        }
        async function main(): Promise<void> {
            const first: Result = { kind: "number", value: 2 };
            const second: Result = { kind: "text", value: "sync" };
            const inferred = [first, second];
            print(inferred);
            const pending: Promise<Result>[] = [number(), text()];
            const fromVariable = await Promise.all(pending);
            print(fromVariable);
            const fromLiteral = await Promise.all([number(), text()]);
            print(fromLiteral);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "inferred_union_array_metadata"),
        "12\nsync!\n11\nasync!\n11\nasync!\n"
    );
}

#[test]
fn compiles_and_runs_a_napi_addon_with_async_work() {
    let dir =
        std::env::temp_dir().join(format!("thaw-hir-codegen-test-napi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let addon_c = dir.join("addon.c");
    let addon = dir.join("addon.node");
    std::fs::write(&addon_c, r#"
        #include <stddef.h>
        #include <stdio.h>
        #include <stdlib.h>
        typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
        typedef void* napi_async_work;
        typedef int napi_status;
        extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
        extern napi_status napi_get_value_double(napi_env, napi_value, double*);
        extern napi_status napi_create_double(napi_env, double, napi_value*);
        extern napi_status napi_create_function(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, napi_value*);
        extern napi_status napi_set_named_property(napi_env, napi_value, const char*, napi_value);
        extern napi_status napi_throw_error(napi_env, const char*, const char*);
        extern napi_status napi_get_undefined(napi_env, napi_value*);
        extern napi_status napi_create_async_work(napi_env, napi_value, napi_value, void (*)(napi_env, void*), void (*)(napi_env, napi_status, void*), void*, napi_async_work*);
        extern napi_status napi_queue_async_work(napi_env, napi_async_work);
        extern napi_status napi_delete_async_work(napi_env, napi_async_work);
        extern napi_status napi_call_function(napi_env, napi_value, napi_value, size_t, const napi_value*, napi_value*);
        struct async_data { napi_env env; napi_async_work work; napi_value callback; int answer; };
        static napi_value add(napi_env env, napi_callback_info info) {
            size_t argc = 2; napi_value argv[2]; double a, b; napi_value result;
            napi_get_cb_info(env, info, &argc, argv, 0, 0);
            napi_get_value_double(env, argv[0], &a); napi_get_value_double(env, argv[1], &b);
            napi_create_double(env, a + b, &result); return result;
        }
        static napi_value fail(napi_env env, napi_callback_info info) {
            (void)info; napi_throw_error(env, 0, "native addon failed"); return 0;
        }
        static void execute_async(napi_env env, void* raw) {
            (void)env; ((struct async_data*)raw)->answer = 42;
        }
        static void complete_async(napi_env env, napi_status status, void* raw) {
            struct async_data* data = raw;
            printf("async %d status %d\n", data->answer, status);
            napi_value args[2], ignored;
            napi_get_undefined(env, &args[0]);
            napi_create_double(env, data->answer, &args[1]);
            napi_call_function(env, args[0], data->callback, 2, args, &ignored);
            napi_call_function(env, args[0], data->callback, 2, args, &ignored);
            napi_delete_async_work(env, data->work);
            free(data);
        }
        static napi_value schedule(napi_env env, napi_callback_info info) {
            struct async_data* data = calloc(1, sizeof(*data));
            size_t argc = 1;
            napi_value result;
            napi_get_cb_info(env, info, &argc, &data->callback, 0, 0);
            data->env = env;
            napi_create_async_work(env, 0, 0, execute_async, complete_async, data, &data->work);
            napi_queue_async_work(env, data->work);
            napi_get_undefined(env, &result);
            return result;
        }
        __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
            napi_value add_fn, fail_fn, schedule_fn;
            napi_create_function(env, "add", 3, add, 0, &add_fn);
            napi_create_function(env, "fail", 4, fail, 0, &fail_fn);
            napi_create_function(env, "schedule", 8, schedule, 0, &schedule_fn);
            napi_set_named_property(env, exports, "add", add_fn);
            napi_set_named_property(env, exports, "fail", fail_fn);
            napi_set_named_property(env, exports, "schedule", schedule_fn); return exports;
        }
    "#).unwrap();
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&addon_c)
        .arg("-o")
        .arg(&addon)
        .status()
        .unwrap()
        .success());

    let source = format!(
        r#"
        function main(): void {{
            loadNativeAddon("{}");
            console.log(Number(callNativeAddon("add", JSON.parse("[20,22]"))));
            try {{
                const ignored = callNativeAddon("fail", JSON.parse("[]"));
            }} catch (error) {{
                console.log(error);
            }}
            const scheduled = callNativeAddonWithCallback(
                "schedule",
                JSON.parse("[]"),
                (error: Json, result: Json): Json => {{
                    console.log(Number(result));
                    return result;
                }}
            );
        }}
    "#,
        addon.display()
    );
    let module = thaw_parser::parse_typescript(&source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "napi_addon");
    compiler.compile_program(&program).unwrap();
    let obj = dir.join("out.o");
    let exe = dir.join("out");
    compiler.write_object_file(&obj).unwrap();
    let arena = build_staticlib("thaw-arena");
    let std = build_staticlib("thaw-std");
    let runtime = build_staticlib("thaw-runtime");
    let napi = build_staticlib("thaw-napi");
    assert!(Command::new("cc")
        .arg(&obj)
        .arg(&arena)
        .arg(&std)
        .arg(&runtime)
        .arg(&napi)
        .args(["-lm", "-ldl", "-lpthread", "-Wl,--export-dynamic", "-o"])
        .arg(&exe)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "42\nnative addon failed\nasync 42 status 0\n42\n42\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Same mechanism, but through the Lambda `handler` entry point instead
/// of `main` (`emit_lambda_entry` has its own copy of the
/// `call_module_init_if_present` call, see hir_codegen.rs).
#[test]
fn runs_module_init_before_lambda_handler() {
    let source = r#"
        function __thaw_module_init(): void {
            loadScript("function greet() { return 'hi from registry'; }");
        }

        function handler(event: string): string {
            const result = callDynamic("greet", JSON.parse("[]"));
            return String(result);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "module_init_lambda");
    compiler.compile_program(&program).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-module_init_lambda-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    compiler.write_object_file(&obj_path).unwrap();

    let arena_lib = build_staticlib("thaw-arena");
    let runtime_lib = build_staticlib("thaw-runtime");
    let std_lib = build_staticlib("thaw-std");
    let quickjs_lib = build_staticlib("thaw-quickjs");

    let link_status = Command::new("cc")
        .arg(&obj_path)
        .arg(&arena_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&quickjs_lib)
        .arg("-lm")
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    // Same mock Lambda Runtime API protocol as
    // `compiles_and_runs_a_lambda_handler_against_a_mock_runtime_api`:
    // bind a real TCP listener, tell the binary about it via
    // `AWS_LAMBDA_RUNTIME_API`, and check the response it posts back.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = conn.read(&mut buf).unwrap();
        let body = "\"ping\"";
        let response = format!(
            "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: test-req-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        conn.write_all(response.as_bytes()).unwrap();
        drop(conn);

        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        conn.read_to_end(&mut buf).unwrap();
        tx.send(String::from_utf8_lossy(&buf).into_owned()).unwrap();

        let response =
            "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        conn.write_all(response.as_bytes()).unwrap();
    });

    let mut child = Command::new(&exe_path)
        .env("AWS_LAMBDA_RUNTIME_API", &addr)
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn compiled Lambda handler binary");

    let post_request = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("handler never posted a response to the mock runtime API");
    server.join().unwrap();

    let _ = child.kill();
    let _ = child.wait_with_output();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/response"));
    assert!(
        post_request.ends_with("hi from registry"),
        "response body should be the module-init-loaded function's result, got: {post_request}"
    );
}

/// Async functions without an explicit suspension still use the uniform
/// Promise-handle ABI and resolve their completion in the initial state.
#[test]
fn compiles_async_functions_without_explicit_suspension() {
    let source = r#"
        async function computeStage(): Promise<string> {
            const s: string = process.env.STAGE;
            return s;
        }

        async function addAsync(a: number, b: number): Promise<number> {
            return a + b;
        }

        async function main(): Promise<void> {
            const stage: string = await computeStage();
            console.log(stage);
            const sum: number = await addAsync(2, 3);
            console.log(sum);
        }
    "#;
    assert_eq!(
        compile_and_run_with_env(source, "async_v1", &[("STAGE", "prod")]),
        "prod\n5\n"
    );
}

#[test]
fn await_sleep_is_driven_by_the_runtime_event_loop() {
    let source = r#"
        async function main(): Promise<void> {
            console.log("before");
            await sleep(1);
            console.log("after");
        }
    "#;
    assert_eq!(compile_and_run(source, "await_sleep"), "before\nafter\n");
}

#[test]
fn frame_split_async_main_has_resume_function_and_multiple_states() {
    let source = r#"
        async function main(): Promise<void> {
            console.log("state-0");
            await sleep(1);
            console.log("state-1");
            await sleep(1);
            console.log("state-2");
        }
    "#;

    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "frame_split_ir");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("define ptr @thaw_user_main()"));
    assert!(ir.contains("define internal void @thaw_user_main.resume"));
    assert!(ir.contains("state_1"));
    assert!(ir.contains("state_2"));

    assert_eq!(
        compile_and_run(source, "frame_split_multiple_awaits"),
        "state-0\nstate-1\nstate-2\n"
    );
}

#[test]
fn frame_split_preserves_and_mutates_locals_across_awaits() {
    let source = r#"
        async function main(): Promise<void> {
            let value: number = 40;
            await sleep(1);
            value = value + 2;
            await sleep(1);
            console.log(value);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "frame_local_slots");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("frame_value"));
    assert_eq!(compile_and_run(source, "frame_local_slots"), "42\n");
}

#[test]
fn frame_splits_non_main_async_function() {
    let source = r#"
        async function compute(): Promise<number> {
            await sleep(1);
            return 42;
        }

        function main(): void {
            console.log("main");
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "general_async_frame");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("define ptr @compute()"));
    assert!(ir.contains("define internal void @compute.resume"));
    assert_eq!(compile_and_run(source, "general_async_frame"), "main\n");
}

#[test]
fn frame_split_awaits_user_async_function_and_reads_result() {
    let source = r#"
        async function compute(): Promise<number> {
            await sleep(1);
            return 42;
        }

        async function main(): Promise<void> {
            const value: number = await compute();
            console.log(value);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "await_async_result");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("call ptr @compute()"));
    assert!(ir.contains("__thaw_await_0"));
    assert_eq!(compile_and_run(source, "await_async_result"), "42\n");
}

#[test]
fn frame_split_promise_all_preserves_order_and_supports_empty_arrays() {
    let source = r#"
        async function delayed(value: number, milliseconds: number): Promise<number> {
            await sleep(milliseconds);
            return value;
        }

        async function main(): Promise<void> {
            const values: number[] = await Promise.all([
                delayed(1, 45),
                delayed(2, 5),
                delayed(3, 20)
            ]);
            console.log(values[0]);
            console.log(values[1]);
            console.log(values[2]);
            console.log(values.length);
            const empty: number[] = await Promise.all([]);
            console.log(empty.length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_order"),
        "1\n2\n3\n3\n0\n"
    );
}

#[test]
fn frame_split_promise_all_supports_strings_and_booleans() {
    let source = r#"
        async function word(value: string, milliseconds: number): Promise<string> {
            await sleep(milliseconds);
            return value;
        }

        async function flag(value: boolean, milliseconds: number): Promise<boolean> {
            await sleep(milliseconds);
            return value;
        }

        async function main(): Promise<void> {
            const words: string[] = await Promise.all([
                word("first", 20),
                word("second", 1)
            ]);
            const flags: boolean[] = await Promise.all([
                flag(true, 15),
                flag(false, 1)
            ]);
            console.log(words[0]);
            console.log(words[1]);
            console.log(flags[0]);
            console.log(flags[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_typed_scalars"),
        "first\nsecond\ntrue\nfalse\n"
    );
}

#[test]
fn frame_split_promise_all_supports_aggregate_values() {
    let source = r#"
        interface Item { value: number; }

        async function item(value: number, milliseconds: number): Promise<Item> {
            await sleep(milliseconds);
            return { value: value };
        }

        async function row(value: number, milliseconds: number): Promise<number[]> {
            await sleep(milliseconds);
            return [value, value + 1];
        }

        async function main(): Promise<void> {
            const items: Item[] = await Promise.all([item(7, 15), item(9, 1)]);
            const rows: number[][] = await Promise.all([row(3, 12), row(5, 1)]);
            console.log(items[0].value);
            console.log(items[1].value);
            console.log(rows[0][1]);
            console.log(rows[1][0]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_aggregate_values"),
        "7\n9\n4\n5\n"
    );
}

#[test]
fn frame_split_promise_all_accepts_a_promise_array_variable() {
    let source = r#"
        async function delayed(value: number, milliseconds: number): Promise<number> {
            await sleep(milliseconds);
            return value;
        }

        async function main(): Promise<void> {
            const pending: Promise<number>[] = [delayed(4, 20), delayed(6, 1)];
            const values: number[] = await Promise.all(pending);
            console.log(values[0]);
            console.log(values[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_array_variable"),
        "4\n6\n"
    );
}

#[test]
fn frame_split_promise_all_supports_heterogeneous_tuples() {
    let source = r#"
        async function numberValue(): Promise<number> {
            await sleep(15);
            return 12;
        }
        async function stringValue(): Promise<string> {
            await sleep(1);
            return "thaw";
        }
        async function boolValue(): Promise<boolean> {
            await sleep(5);
            return true;
        }

        async function main(): Promise<void> {
            const values: [number, string, boolean] = await Promise.all([
                numberValue(), stringValue(), boolValue()
            ]);
            console.log(values[0]);
            console.log(values[1]);
            console.log(values[2]);
            console.log(values.length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_heterogeneous_tuple"),
        "12\nthaw\ntrue\n3\n"
    );
}

#[test]
fn frame_split_promise_all_tuple_supports_aggregate_members() {
    let source = r#"
        interface Item { value: number; }
        async function item(): Promise<Item> {
            await sleep(8);
            return { value: 21 };
        }
        async function row(): Promise<number[]> {
            await sleep(1);
            return [30, 31];
        }
        async function main(): Promise<void> {
            const values: [Item, number[]] = await Promise.all([item(), row()]);
            console.log(values[0].value);
            console.log(values[1][1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_tuple_aggregates"),
        "21\n31\n"
    );
}

#[test]
fn frame_split_promise_all_rejection_enters_nearest_catch() {
    let source = r#"
        async function succeeds(): Promise<number> {
            await sleep(10);
            return 1;
        }

        async function fails(): Promise<number> {
            await sleep(2);
            throw "joined failure";
        }

        async function main(): Promise<void> {
            try {
                const values: number[] = await Promise.all([succeeds(), fails()]);
                console.log(values[0]);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_rejection"),
        "joined failure\n"
    );
}

#[test]
fn frame_split_promise_all_tuple_rejection_enters_nearest_catch() {
    let source = r#"
        async function succeeds(): Promise<number> {
            await sleep(15);
            return 1;
        }
        async function fails(): Promise<string> {
            await sleep(1);
            throw "tuple failure";
        }
        async function main(): Promise<void> {
            try {
                const values: [number, string] = await Promise.all([succeeds(), fails()]);
                console.log(values[0]);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_tuple_rejection"),
        "tuple failure\n"
    );
}

#[test]
fn frame_split_promise_race_uses_the_first_completion() {
    let source = r#"
        async function delayed(value: number, milliseconds: number): Promise<number> {
            await sleep(milliseconds);
            return value;
        }
        async function main(): Promise<void> {
            const value: number = await Promise.race([
                delayed(1, 30), delayed(2, 2), delayed(3, 15)
            ]);
            console.log(value);
            const pending: Promise<number>[] = [delayed(4, 20), delayed(5, 1)];
            const fromArray: number = await Promise.race(pending);
            console.log(fromArray);
        }
    "#;
    assert_eq!(compile_and_run(source, "promise_race_order"), "2\n5\n");
}

#[test]
fn frame_split_promise_race_supports_native_value_shapes() {
    let source = r#"
        interface Item { value: number; }
        async function word(value: string, ms: number): Promise<string> {
            await sleep(ms); return value;
        }
        async function flag(value: boolean, ms: number): Promise<boolean> {
            await sleep(ms); return value;
        }
        async function item(value: number, ms: number): Promise<Item> {
            await sleep(ms); return { value: value };
        }
        async function row(value: number, ms: number): Promise<number[]> {
            await sleep(ms); return [value, value + 1];
        }
        async function main(): Promise<void> {
            const text: string = await Promise.race([word("slow", 15), word("fast", 1)]);
            const yes: boolean = await Promise.race([flag(false, 15), flag(true, 1)]);
            const object: Item = await Promise.race([item(6, 15), item(7, 1)]);
            const values: number[] = await Promise.race([row(8, 15), row(9, 1)]);
            console.log(text);
            console.log(yes);
            console.log(object.value);
            console.log(values[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_race_native_shapes"),
        "fast\ntrue\n7\n10\n"
    );
}

#[test]
fn frame_split_promise_race_rejection_enters_nearest_catch() {
    let source = r#"
        async function succeeds(): Promise<number> {
            await sleep(20); return 1;
        }
        async function fails(): Promise<number> {
            await sleep(1); throw "race failure";
        }
        async function main(): Promise<void> {
            try {
                const value: number = await Promise.race([succeeds(), fails()]);
                console.log(value);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_race_rejection"),
        "race failure\n"
    );
}

#[test]
fn frame_split_promise_any_ignores_rejections_and_accepts_array_variables() {
    let source = r#"
        async function succeeds(value: number, ms: number): Promise<number> {
            await sleep(ms); return value;
        }
        async function fails(ms: number): Promise<number> {
            await sleep(ms); throw "ignored";
        }
        async function main(): Promise<void> {
            const value: number = await Promise.any([
                fails(1), succeeds(4, 20), succeeds(5, 3)
            ]);
            console.log(value);
            const pending: Promise<number>[] = [fails(1), succeeds(6, 3)];
            const fromArray: number = await Promise.any(pending);
            console.log(fromArray);
        }
    "#;
    assert_eq!(compile_and_run(source, "promise_any_success"), "5\n6\n");
}

#[test]
fn frame_split_promise_any_supports_native_value_shapes() {
    let source = r#"
        interface Item { value: number; }
        async function word(value: string, ms: number): Promise<string> {
            await sleep(ms); return value;
        }
        async function flag(value: boolean, ms: number): Promise<boolean> {
            await sleep(ms); return value;
        }
        async function item(value: number, ms: number): Promise<Item> {
            await sleep(ms); return { value: value };
        }
        async function row(value: number, ms: number): Promise<number[]> {
            await sleep(ms); return [value, value + 1];
        }
        async function main(): Promise<void> {
            const text: string = await Promise.any([word("slow", 10), word("fast", 1)]);
            const yes: boolean = await Promise.any([flag(false, 10), flag(true, 1)]);
            const object: Item = await Promise.any([item(7, 10), item(8, 1)]);
            const values: number[] = await Promise.any([row(9, 10), row(10, 1)]);
            console.log(text);
            console.log(yes);
            console.log(object.value);
            console.log(values[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_any_native_shapes"),
        "fast\ntrue\n8\n11\n"
    );
}

#[test]
fn frame_split_promise_any_all_rejected_enters_nearest_catch() {
    let source = r#"
        async function fails(message: string, ms: number): Promise<number> {
            await sleep(ms); throw message;
        }
        async function main(): Promise<void> {
            try {
                const value: number = await Promise.any([
                    fails("first", 1), fails("second", 3)
                ]);
                console.log(value);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_any_all_rejected"),
        "All promises were rejected\n"
    );
}

#[test]
fn frame_split_promise_all_settled_preserves_order_and_never_rejects() {
    let source = r#"
        async function succeeds(value: number, ms: number): Promise<number> {
            await sleep(ms); return value;
        }
        async function fails(message: string, ms: number): Promise<number> {
            await sleep(ms); throw message;
        }
        async function main(): Promise<void> {
            const results: { status: string; value: number; reason: string }[] =
                await Promise.allSettled([
                    succeeds(1, 15), fails("broken", 1), succeeds(3, 5)
                ]);
            console.log(results[0].status);
            console.log(results[0].value);
            console.log(results[0].reason);
            console.log(results[1].status);
            console.log(results[1].reason);
            console.log(results[2].status);
            console.log(results[2].value);
            console.log(results.length);
            const empty: { status: string; value: number; reason: string }[] =
                await Promise.allSettled([]);
            console.log(empty.length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_settled_order"),
        "fulfilled\n1\n\nrejected\nbroken\nfulfilled\n3\n3\n0\n"
    );
}

#[test]
fn frame_split_promise_all_settled_accepts_array_variables() {
    let source = r#"
        async function value(value: number, ms: number): Promise<number> {
            await sleep(ms); return value;
        }
        async function main(): Promise<void> {
            const pending: Promise<number>[] = [value(4, 10), value(5, 1)];
            const results: { status: string; value: number; reason: string }[] =
                await Promise.allSettled(pending);
            console.log(results[0].value);
            console.log(results[1].value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_settled_array"),
        "4\n5\n"
    );
}

#[test]
fn promise_combinators_accept_array_literal_spreads() {
    let source = r#"
        async function succeeds(value: number, ms: number): Promise<number> {
            await sleep(ms); return value;
        }
        async function fails(message: string, ms: number): Promise<number> {
            await sleep(ms); throw message;
        }
        function allBatch(): Promise<number>[] {
            console.log("all-batch");
            return [succeeds(2, 2), succeeds(3, 1)];
        }
        function failedBatch(): Promise<number>[] {
            console.log("failed-batch");
            return [fails("no", 1)];
        }
        async function main(): Promise<void> {
            const all: number[] = await Promise.all([
                succeeds(1, 1), ...allBatch()
            ]);
            console.log(all[0]); console.log(all[1]); console.log(all[2]);
            const settled: { status: string; value: number; reason: string }[] =
                await Promise.allSettled([...failedBatch(), succeeds(4, 1)]);
            console.log(settled[0].status); console.log(settled[0].reason);
            console.log(settled[1].status); console.log(settled[1].value);
            const raced: number = await Promise.race([
                ...[succeeds(9, 10)], succeeds(5, 1)
            ]);
            console.log(raced);
            const any: number = await Promise.any([
                ...[fails("skip", 1)], succeeds(7, 2)
            ]);
            console.log(any);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_combinator_spreads"),
        "all-batch\n1\n2\n3\nfailed-batch\nrejected\nno\nfulfilled\n4\n5\n7\n"
    );
}

#[test]
fn frame_split_promise_all_settled_supports_native_value_shapes() {
    let source = r#"
        interface Item { value: number; }
        async function word(): Promise<string> { await sleep(1); return "text"; }
        async function flag(): Promise<boolean> { await sleep(1); return true; }
        async function item(): Promise<Item> { await sleep(1); return { value: 8 }; }
        async function row(): Promise<number[]> { await sleep(1); return [9, 10]; }
        async function main(): Promise<void> {
            const words: { status: string; value: string; reason: string }[] =
                await Promise.allSettled([word()]);
            const flags: { status: string; value: boolean; reason: string }[] =
                await Promise.allSettled([flag()]);
            const items: { status: string; value: Item; reason: string }[] =
                await Promise.allSettled([item()]);
            const rows: { status: string; value: number[]; reason: string }[] =
                await Promise.allSettled([row()]);
            console.log(words[0].value);
            console.log(flags[0].value);
            console.log(items[0].value.value);
            console.log(rows[0].value[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_all_settled_shapes"),
        "text\ntrue\n8\n10\n"
    );
}

#[test]
fn promise_combinators_preserve_union_values_and_discriminants() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        async function number(delay: number): Promise<Result> {
            await sleep(delay);
            return { kind: "number", value: 1 };
        }
        async function text(delay: number): Promise<Result> {
            await sleep(delay);
            return { kind: "text", value: "async" };
        }
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        async function main(): Promise<void> {
            const racing: Promise<Result>[] = [text(10), number(1)];
            const raced = await Promise.race(racing);
            print(raced);
            const any = await Promise.any([text(1), number(10)]);
            print(any);
            const settled = await Promise.allSettled([number(1), text(1)]);
            for (const item of settled) {
                if (item.status === "fulfilled") print(item.value);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_union_combinators"),
        "11\nasync!\n11\nasync!\n"
    );
}

#[test]
fn promise_chains_preserve_union_values_and_discriminants() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        async function value(): Promise<Result> {
            await sleep(1);
            return { kind: "number", value: 1 };
        }
        async function failure(): Promise<Result> {
            await sleep(1);
            throw "failed";
        }
        function transform(item: Result): Result {
            if (item.kind === "number") return { kind: "text", value: "then" };
            return item;
        }
        function recover(reason: string): Result {
            return { kind: "text", value: reason };
        }
        function print(item: Result): void {
            if (item.kind === "number") console.log(item.value + 10);
            else console.log(item.value + "!");
        }
        async function main(): Promise<void> {
            const transformed = await value().then(transform);
            print(transformed);
            const recovered = await failure().catch(recover);
            print(recovered);
            const preserved = await value().finally(() => console.log("finally"));
            print(preserved);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_union_chains"),
        "then!\nfailed!\nfinally\n11\n"
    );
}

#[test]
fn frame_split_async_functions_return_objects_arrays_and_tuples_from_branches() {
    let source = r#"
        interface Item { value: number; }
        async function objectValue(first: boolean): Promise<Item> {
            await sleep(1);
            if (first) { return { value: 11 }; }
            return { value: 12 };
        }
        async function arrayValue(first: boolean): Promise<number[]> {
            await sleep(1);
            if (first) { return [21, 22]; }
            return [23, 24];
        }
        async function tupleValue(first: boolean): Promise<[number, string]> {
            await sleep(1);
            if (first) { return [31, "first"]; }
            return [32, "second"];
        }
        async function main(): Promise<void> {
            const object: Item = await objectValue(false);
            const array: number[] = await arrayValue(true);
            const tuple: [number, string] = await tupleValue(false);
            console.log(object.value);
            console.log(array[1]);
            console.log(tuple[0]);
            console.log(tuple[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_aggregate_branch_returns"),
        "12\n22\n32\nsecond\n"
    );
}

#[test]
fn frame_split_extracts_awaits_from_arguments_and_literals_left_to_right() {
    let source = r#"
        interface Pair { left: number; right: number; }
        async function delayed(value: number, ms: number): Promise<number> {
            await sleep(ms); return value;
        }
        function combine(left: number, right: number): number {
            return left * 10 + right;
        }
        async function main(): Promise<void> {
            const combined: number = combine(
                await delayed(1, 3), await delayed(2, 1)
            );
            const values: number[] = [
                await delayed(3, 2), await delayed(4, 1)
            ];
            const pair: Pair = {
                left: await delayed(5, 2),
                right: await delayed(6, 1)
            };
            console.log(combined);
            console.log(values[0] * 10 + values[1]);
            console.log(pair.left * 10 + pair.right);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "awaits_in_arguments_and_literals"),
        "12\n34\n56\n"
    );
}

#[test]
fn frame_split_preserves_synchronous_call_arguments_before_await() {
    let source = r#"
        interface Trace { value: string; }
        function record(trace: Trace, label: string, value: number): number {
            trace.value = trace.value + label;
            return value;
        }
        async function delayed(trace: Trace, label: string, value: number): Promise<number> {
            trace.value = trace.value + label;
            await sleep(1);
            return value;
        }
        function combine(first: number, second: number, third: number): number {
            return first * 100 + second * 10 + third;
        }
        function consume(trace: Trace, first: number, second: number): void {
            trace.value = trace.value + String(first) + String(second);
        }
        async function main(): Promise<void> {
            const trace: Trace = { value: "" };
            const value: number = combine(
                record(trace, "a", 1), await delayed(trace, "b", 2), record(trace, "c", 3)
            );
            consume(trace, record(trace, "d", 4), await delayed(trace, "e", 5));
            consume(trace, record(trace, "f", 6), 1 + await delayed(trace, "g", 6));
            console.log(trace.value);
            console.log(value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "call_argument_order_across_await"),
        "abcde45fg67\n123\n"
    );
}

#[test]
fn frame_split_preserves_binary_left_operand_before_nested_await() {
    let source = r#"
        interface Trace { value: string; }
        function record(trace: Trace, label: string, value: number): number {
            trace.value = trace.value + label;
            return value;
        }
        async function delayed(trace: Trace, label: string, value: number): Promise<number> {
            trace.value = trace.value + label;
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const trace: Trace = { value: "" };
            const sum: number = record(trace, "a", 20)
                + await delayed(trace, "b", 22);
            const comparison: boolean = record(trace, "c", 1)
                < await delayed(trace, "d", 2);
            const nested: number = 2 * (record(trace, "e", 3)
                + await delayed(trace, "f", 4));
            console.log(trace.value);
            console.log(sum);
            console.log(comparison);
            console.log(nested);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "binary_order_across_nested_await"),
        "abcdef\n42\ntrue\n14\n"
    );
}

#[test]
fn frame_split_preserves_array_elements_before_nested_await() {
    let source = r#"
        interface Trace { value: string; }
        function record(trace: Trace, label: string, value: number): number {
            trace.value = trace.value + label;
            return value;
        }
        async function delayed(trace: Trace, label: string, value: number): Promise<number> {
            trace.value = trace.value + label;
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const trace: Trace = { value: "" };
            const values: number[] = [
                record(trace, "a", 1),
                await delayed(trace, "b", 2),
                record(trace, "c", 3),
                record(trace, "d", 4) + await delayed(trace, "e", 1)
            ];
            console.log(trace.value);
            console.log(values.join("-"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_order_across_nested_await"),
        "abcde\n1-2-3-5\n"
    );
}

#[test]
fn frame_split_preserves_object_fields_before_nested_await() {
    let source = r#"
        interface Trace { value: string; }
        interface Values { first: number; second: number; third: number; }
        function record(trace: Trace, label: string, value: number): number {
            trace.value = trace.value + label;
            return value;
        }
        async function delayed(trace: Trace, label: string, value: number): Promise<number> {
            trace.value = trace.value + label;
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const trace: Trace = { value: "" };
            const values: Values = {
                first: record(trace, "a", 1),
                second: await delayed(trace, "b", 2),
                third: record(trace, "c", 3) + await delayed(trace, "d", 1)
            };
            console.log(trace.value);
            console.log(values.first + values.second + values.third);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_order_across_nested_await"),
        "abcd\n7\n"
    );
}

#[test]
fn frame_split_returns_from_deep_async_loop_try_and_block_scopes() {
    let source = r#"
        async function delayed(value: number): Promise<number> {
            await sleep(1); return value;
        }
        async function find(): Promise<number> {
            let index: number = 0;
            while (index < 4) {
                try {
                    const current: number = await delayed(index);
                    if (current === 2) {
                        const answer: number = await delayed(current + 40);
                        return answer;
                    }
                } catch (error) {
                    return 0;
                }
                index = index + 1;
            }
            return 1;
        }
        async function main(): Promise<void> {
            const value: number = await find();
            console.log(value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "deep_async_return_control_flow"),
        "42\n"
    );
}

#[test]
fn frame_split_awaits_promises_stored_in_variables_arguments_and_objects() {
    let source = r#"
        interface Holder { pending: Promise<number>; }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }
        async function consume(pending: Promise<number>): Promise<number> {
            return await pending;
        }
        async function main(): Promise<void> {
            const pending: Promise<number> = delayed(41);
            const holder: Holder = { pending: delayed(42) };
            console.log(await consume(pending));
            console.log(await holder.pending);
        }
    "#;
    assert_eq!(compile_and_run(source, "stored_promise_values"), "41\n42\n");
}

#[test]
fn promise_void_continuations_run_and_settle() {
    let source = r#"
        async function main(): Promise<void> {
            await new Promise<void>((resolve, reject) => resolve())
                .then(() => console.log("then"));
            await new Promise<void>((resolve, reject) => reject("failure"))
                .catch(error => console.log(error));
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_void_continuations"),
        "then\nfailure\ndone\n"
    );
}

#[test]
fn promise_then_transforms_all_native_value_shapes() {
    let source = r#"
        interface Item { value: number; }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const text: string = await new Promise<number>((resolve, reject) => {
                resolve(1);
            }).then(value => "ready");
            const flag: boolean = await new Promise<number>((resolve, reject) => {
                resolve(1);
            }).then(value => true);
            const item: Item = await new Promise<number>((resolve, reject) => {
                resolve(9);
            }).then(value => ({ value: value }));
            const values: number[] = await new Promise<number>((resolve, reject) => {
                resolve(10);
            }).then(value => [value, value + 1]);
            const tuple: [number, string] = await new Promise<number>((resolve, reject) => {
                resolve(12);
            }).then(value => [value, "tuple"]);
            const flattened: number = await new Promise<number>((resolve, reject) => {
                resolve(20);
            }).then(value => delayed(value + 1));
            const recovered: number = await new Promise<number>((resolve, reject) => {
                reject("recover asynchronously");
            }).catch(error => delayed(22));
            console.log(text);
            console.log(flag);
            console.log(item.value);
            console.log(values[1]);
            console.log(tuple[0]);
            console.log(tuple[1]);
            console.log(flattened);
            console.log(recovered);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_chain_shapes"),
        "ready\ntrue\n9\n11\n12\ntuple\n21\n22\n"
    );
}

#[test]
fn promise_finally_preserves_settlement_and_waits_for_promises() {
    let source = r#"
        async function cleanup(): Promise<void> {
            await sleep(1);
            console.log("async cleanup");
        }
        async function main(): Promise<void> {
            const fulfilled: number = await new Promise<number>((resolve, reject) => {
                resolve(41);
            }).finally(() => console.log("fulfilled cleanup"));
            console.log(fulfilled);
            const recovered: number = await new Promise<number>((resolve, reject) => {
                reject("original rejection");
            }).finally(() => {
                console.log("rejected cleanup");
            }).catch(error => {
                console.log(error);
                return 42;
            });
            console.log(recovered);
            const waited: number = await new Promise<number>((resolve, reject) => {
                resolve(43);
            }).finally(() => cleanup());
            console.log(waited);
            const replaced: number = await new Promise<number>((resolve, reject) => {
                resolve(1);
            }).finally(() => {
                throw "finally failure";
            }).catch(error => {
                console.log(error);
                return 44;
            });
            console.log(replaced);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_finally"),
        "fulfilled cleanup\n41\nrejected cleanup\noriginal rejection\n42\nasync cleanup\n43\nfinally failure\n44\n"
    );
}

#[test]
fn directly_awaited_finally_uses_async_frame_rejection_handling() {
    let source = r#"
        async function main(): Promise<void> {
            const value: number = await new Promise<number>((resolve, reject) => {
                resolve(41);
            }).finally(() => {
                console.log("fulfilled cleanup");
            });
            console.log(value);
            try {
                await new Promise<number>((resolve, reject) => {
                    reject("finally rejection");
                }).finally(() => console.log("rejected cleanup"));
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "direct_await_finally"),
        "fulfilled cleanup\n41\nrejected cleanup\nfinally rejection\n"
    );
}

#[test]
fn promise_callbacks_accept_named_functions_and_function_variables() {
    let source = r#"
        function executor(
            resolve: (value: number) => void,
            reject: (error: string) => void
        ): void {
            resolve(10);
        }
        function double(value: number): number { return value * 2; }
        function recover(error: string): number {
            console.log(error);
            return 30;
        }
        function cleanup(): void { console.log("named cleanup"); }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const plusOne: (value: number) => number =
                (value: number): number => value + 1;
            const value: number = await new Promise(executor)
                .then(double)
                .then(plusOne)
                .finally(cleanup);
            console.log(value);
            const recovered: number = await new Promise<number>((resolve, reject) => {
                reject("named recovery");
            }).catch(recover);
            console.log(recovered);
            const assimilated: number = await new Promise((resolve, reject) => {
                resolve(delayed(31));
            });
            console.log(assimilated);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "promise_named_callbacks"),
        "named cleanup\n21\nnamed recovery\n30\n31\n"
    );
}

#[test]
fn frame_split_promise_all_composes_with_if_and_while() {
    let source = r#"
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let value: number = 0;
            if (value === 0) {
                value = (await Promise.all([delayed(1)]))[0];
            }
            while (value < 3) {
                value = (await Promise.all([delayed(value + 1)]))[0];
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "promise_all_control_flow"), "3\n");
}

#[test]
fn frame_split_async_function_preserves_arguments_across_await() {
    let source = r#"
        async function compute(a: number, b: number): Promise<number> {
            await sleep(1);
            return a + b;
        }

        async function main(): Promise<void> {
            const value: number = await compute(20, 22);
            console.log(value);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "async_arguments");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("define ptr @compute(double"));
    assert!(ir.contains("frame_a"));
    assert!(ir.contains("frame_b"));
    assert_eq!(compile_and_run(source, "async_arguments"), "42\n");
}

#[test]
fn frame_split_supports_await_inside_expressions() {
    let source = r#"
        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            const value: number = (await compute(20)) + (await compute(21)) + 1;
            console.log(value);
            console.log((await compute(6)) * 7);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "await_in_expression");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("frame___thaw_await_0"));
    assert!(ir.contains("frame___thaw_await_1"));
    assert_eq!(compile_and_run(source, "await_in_expression"), "42\n42\n");
}

#[test]
fn frame_split_supports_await_in_if_condition() {
    let source = r#"
        async function isReady(value: number): Promise<boolean> {
            await sleep(1);
            return value > 0;
        }

        async function main(): Promise<void> {
            if (await isReady(1)) {
                console.log("ready");
            } else {
                console.log("not-ready");
            }
            if (await isReady(0)) {
                console.log("unexpected");
            } else {
                console.log("false-branch");
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "await_if_condition"),
        "ready\nfalse-branch\n"
    );
}

#[test]
fn frame_split_supports_await_inside_if_branches() {
    let source = r#"
        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let value: number = 0;
            if (true) {
                value = await compute(20);
            } else {
                console.log("wrong-else");
                value = await compute(100);
            }
            if (false) {
                console.log("wrong-then");
                await sleep(1);
            } else {
                value = value + (await compute(22));
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "await_if_branches"), "42\n");
}

#[test]
fn frame_split_supports_await_inside_while_loop() {
    let source = r#"
        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let index: number = 0;
            let total: number = 0;
            while (index < 3) {
                await sleep(1);
                total = total + (await compute(index));
                index = index + 1;
            }
            console.log(total);
            console.log(index);
        }
    "#;
    assert_eq!(compile_and_run(source, "await_while_loop"), "3\n3\n");
}

#[test]
fn frame_split_supports_await_and_continue_in_do_while_loop() {
    let source = r#"
        async function main(): Promise<void> {
            let i = 0;
            do {
                await sleep(1);
                i++;
                if (i === 1) continue;
                console.log(i);
            } while (i < 3);
            console.log("done");
        }
    "#;
    assert_eq!(compile_and_run(source, "await_do_while"), "2\n3\ndone\n");
}

#[test]
fn frame_split_supports_await_in_switch_tests_and_cases() {
    let source = r#"
        async function selected(): Promise<string> {
            await sleep(1);
            console.log("selected");
            return "b";
        }
        async function main(): Promise<void> {
            let result = 0;
            switch ("b") {
                case "a": console.log("wrong-a"); break;
                case await selected():
                    console.log("matched");
                    await sleep(1);
                    result = 2;
                default:
                    console.log("fallthrough-default");
                    result = result + 1;
                    break;
                case "never": console.log("wrong-never");
            }
            console.log(result);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "await_switch"),
        "selected\nmatched\nfallthrough-default\n3\n"
    );
}

#[test]
fn frame_split_supports_await_in_string_for_of_loop() {
    let source = r#"
        async function main(): Promise<void> {
            const values: string[] = ["first", "skip", "last"];
            for (const value of values) {
                await sleep(1);
                if (value === "skip") continue;
                console.log(value);
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "await_for_of"),
        "first\nlast\ndone\n"
    );
}

#[test]
fn frame_split_supports_for_await_of_and_rejection_catch() {
    let source = r#"
        async function main(): Promise<void> {
            const values: Promise<number>[] = [
                new Promise<number>((resolve, reject) => resolve(1)),
                new Promise<number>((resolve, reject) => resolve(2)),
                new Promise<number>((resolve, reject) => resolve(3))
            ];
            for await (const value of values) {
                if (value === 2) continue;
                console.log(value);
            }
            let last = 0;
            const assigned: Promise<number>[] = [
                new Promise<number>((resolve, reject) => resolve(4)),
                new Promise<number>((resolve, reject) => resolve(5))
            ];
            for await (last of assigned) {}
            console.log(last);
            for await (const immediate of [6, 7]) {
                console.log(immediate);
            }
            try {
                const failures: Promise<number>[] = [
                    new Promise<number>((resolve, reject) => reject("for await failure"))
                ];
                for await (const value of failures) {
                    console.log(value);
                }
            } catch (error) {
                console.log(error);
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "for_await_of"),
        "1\n3\n5\n6\n7\nfor await failure\ndone\n"
    );
}

#[test]
fn frame_split_supports_await_in_while_condition() {
    let source = r#"
        async function shouldContinue(value: number): Promise<boolean> {
            await sleep(1);
            return value < 3;
        }

        async function main(): Promise<void> {
            let index: number = 0;
            while (await shouldContinue(index)) {
                console.log(index);
                index = index + 1;
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "await_while_condition"),
        "0\n1\n2\ndone\n"
    );
}

#[test]
fn frame_split_supports_deeply_nested_if_awaits() {
    let source = r#"
        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let value: number = 0;
            if (true) {
                if (false) {
                    console.log("wrong-inner-then");
                    value = 100;
                } else {
                    if (true) {
                        value = await compute(42);
                    }
                }
            } else {
                console.log("wrong-outer-else");
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "nested_async_if"), "42\n");
}

#[test]
fn frame_split_supports_await_in_condition_and_branch_body() {
    let source = r#"
        async function ready(value: boolean): Promise<boolean> {
            await sleep(1);
            return value;
        }

        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let value: number = 0;
            if (await ready(true)) {
                value = await compute(42);
            } else {
                value = await compute(100);
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "async_if_condition_body"), "42\n");
}

#[test]
fn frame_split_supports_await_in_deeply_nested_if_condition() {
    let source = r#"
        async function ready(value: boolean): Promise<boolean> {
            await sleep(1);
            return value;
        }

        async function compute(value: number): Promise<number> {
            await sleep(1);
            return value;
        }

        async function main(): Promise<void> {
            let value: number = 0;
            if (true) {
                if (await ready(false)) {
                    value = await compute(100);
                } else {
                    if (await ready(true)) {
                        value = await compute(42);
                    }
                }
            }
            console.log(value);
        }
    "#;
    assert_eq!(compile_and_run(source, "nested_async_if_condition"), "42\n");
}

#[test]
fn frame_split_supports_nested_async_while_loops() {
    let source = r#"
        async function below(value: number, limit: number): Promise<boolean> {
            await sleep(1);
            return value < limit;
        }

        async function main(): Promise<void> {
            let outer: number = 0;
            let inner: number = 0;
            let total: number = 0;
            while (outer < 2) {
                await sleep(1);
                inner = 0;
                while (await below(inner, 3)) {
                    total = total + 1;
                    inner = inner + 1;
                }
                outer = outer + 1;
            }
            console.log(total);
            console.log(outer);
        }
    "#;
    assert_eq!(compile_and_run(source, "nested_async_while"), "6\n2\n");
}

#[test]
fn frame_split_supports_async_try_finally_normal_completion() {
    let source = r#"
        async function main(): Promise<void> {
            try {
                console.log("try");
                await sleep(1);
                console.log("resumed");
            } finally {
                console.log("finally");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_try_finally_normal"),
        "try\nresumed\nfinally\ndone\n"
    );
}

#[test]
fn frame_split_runs_finally_before_async_return() {
    let source = r#"
        async function main(): Promise<void> {
            try {
                await sleep(1);
                console.log("returning");
                return;
            } finally {
                console.log("finally");
            }
            console.log("unreachable");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_try_finally_return"),
        "returning\nfinally\n"
    );
}

#[test]
fn frame_split_propagates_async_rejection_through_await_chain() {
    let source = r#"
        async function fail(): Promise<number> {
            await sleep(1);
            throw "boom";
        }

        async function relay(): Promise<number> {
            const value: number = await fail();
            return value;
        }

        async function main(): Promise<void> {
            console.log("before");
            await relay();
            console.log("unreachable");
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "async_rejection_ir");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("call i8 @thaw_promise_reject"));
    assert!(ir.contains("propagate_rejection"));
    assert_eq!(compile_and_run(source, "async_rejection_chain"), "before\n");
}

#[test]
fn frame_split_catches_throw_after_await_in_same_function() {
    let source = r#"
        async function main(): Promise<void> {
            try {
                console.log("try");
                await sleep(1);
                throw "boom";
                console.log("unreachable");
            } catch (error) {
                console.log(error);
                await sleep(1);
                console.log("caught");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_direct_catch"),
        "try\nboom\ncaught\ndone\n"
    );
}

#[test]
fn frame_split_catches_rejected_child_promise() {
    let source = r#"
        async function fail(): Promise<void> {
            await sleep(1);
            throw "child-boom";
        }

        async function main(): Promise<void> {
            try {
                console.log("before");
                await fail();
                console.log("unreachable");
            } catch (error) {
                console.log(error);
                await sleep(1);
                console.log("caught");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_rejection_catch"),
        "before\nchild-boom\ncaught\ndone\n"
    );
}

#[test]
fn frame_split_rethrows_after_await_in_catch() {
    let source = r#"
        async function fail(): Promise<void> {
            await sleep(1);
            throw "original";
        }

        async function relay(): Promise<void> {
            try {
                await fail();
            } catch (error) {
                console.log(error);
                await sleep(1);
                console.log("rethrowing");
                throw "wrapped";
            }
            console.log("relay-unreachable");
        }

        async function main(): Promise<void> {
            console.log("before");
            await relay();
            console.log("main-unreachable");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_catch_rethrow"),
        "before\noriginal\nrethrowing\n"
    );
}

#[test]
fn frame_split_selects_nearest_catch_in_nested_async_try() {
    let source = r#"
        async function failInner(): Promise<void> {
            await sleep(1);
            throw "inner-error";
        }

        async function failOuter(): Promise<void> {
            await sleep(1);
            throw "outer-error";
        }

        async function main(): Promise<void> {
            try {
                try {
                    await failInner();
                } catch (inner) {
                    console.log(inner);
                    await failOuter();
                    console.log("inner-unreachable");
                }
                console.log("outer-try-unreachable");
            } catch (outer) {
                console.log(outer);
                await sleep(1);
                console.log("outer-caught");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nested_async_try_catch"),
        "inner-error\nouter-error\nouter-caught\ndone\n"
    );
}

#[test]
fn frame_split_supports_arbitrarily_deep_async_try_catch() {
    let source = r#"
        async function failDeep(): Promise<void> {
            await sleep(1);
            throw "deep";
        }

        async function main(): Promise<void> {
            try {
                try {
                    try {
                        await failDeep();
                    } catch (deep) {
                        console.log(deep);
                        await sleep(1);
                        throw "middle";
                    }
                } catch (middle) {
                    console.log(middle);
                    await sleep(1);
                    throw "outer";
                }
            } catch (outer) {
                console.log(outer);
                await sleep(1);
                console.log("caught");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "deep_async_try_catch"),
        "deep\nmiddle\nouter\ncaught\ndone\n"
    );
}

#[test]
fn frame_split_supports_locals_and_return_inside_async_branch() {
    let source = r#"
        async function value(): Promise<number> {
            await sleep(1);
            return 42;
        }

        async function choose(): Promise<number> {
            if (1 < 2) {
                const answer = await value();
                return answer;
            }
            return 0;
        }

        async function main(): Promise<void> {
            const result = await choose();
            console.log(result);
        }
    "#;
    assert_eq!(compile_and_run(source, "async_branch_local_return"), "42\n");
}

#[test]
fn frame_split_supports_try_inside_async_if_and_while() {
    let source = r#"
        async function main(): Promise<void> {
            let i = 0;
            if (1 < 2) {
                try {
                    const label = "if-error";
                    await sleep(1);
                    throw label;
                } catch (error) {
                    console.log(error);
                }
            }
            while (i < 1) {
                try {
                    const label = "while-error";
                    await sleep(1);
                    throw label;
                } catch (error) {
                    console.log(error);
                }
                i = i + 1;
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_try_in_if_while"),
        "if-error\nwhile-error\ndone\n"
    );
}

#[test]
fn frame_split_supports_async_if_and_while_inside_try() {
    let source = r#"
        async function main(): Promise<void> {
            let i = 0;
            try {
                if (1 < 2) {
                    const prefix = "branch";
                    await sleep(1);
                    console.log(prefix);
                }
                while (i < 2) {
                    const current = i;
                    await sleep(1);
                    i = current + 1;
                    if (i > 1) {
                        throw "loop-error";
                    }
                }
                console.log("unreachable");
            } catch (error) {
                console.log(error);
                await sleep(1);
                console.log("caught");
            }
            console.log("done");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "async_if_while_in_try"),
        "branch\nloop-error\ncaught\ndone\n"
    );
}

#[test]
fn frame_split_await_fetch_uses_nonblocking_http_promise() {
    let source = r#"
        async function main(): Promise<void> {
            const body = await fetch("http://127.0.0.1:8080/data");
            console.log(body);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "async_fetch");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("call ptr @thaw_http_get_async"));
    assert!(ir.contains("define internal void @thaw_user_main.resume"));
    assert!(!ir.contains("call ptr @thaw_fetch_get"));
}

/// Full pipeline: `fetch` a JSON body from a real (mock) HTTP server,
/// `JSON.parse` it, read fields (both `.field` and `[i]`), and convert
/// them to concrete types with `Number`/`String`/`Boolean`.
#[test]
fn compiles_fetch_and_json_parsing() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = conn.read(&mut buf).unwrap();
        let body = r#"{"name": "thaw", "active": true, "tags": ["fast", "native"]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        conn.write_all(response.as_bytes()).unwrap();
    });

    let source = format!(
        r#"
        async function main(): Promise<void> {{
            const text: string = await fetch("http://{addr}/");
            const data = JSON.parse(text);
            console.log(String(data.name));
            console.log(Boolean(data.active));
            console.log(Number(data.tags.length));
            console.log(String(data.tags[0]));
            console.log(JSON.stringify(data));
        }}
    "#
    );

    let output = compile_and_run(&source, "fetch_json");
    server.join().unwrap();

    let mut lines = output.lines();
    assert_eq!(lines.next(), Some("thaw"));
    assert_eq!(lines.next(), Some("true"));
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("fast"));
    let reparsed: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(
        reparsed,
        serde_json::json!({"name": "thaw", "active": true, "tags": ["fast", "native"]})
    );
}

/// Full pipeline test for the Lambda entry point: a `handler(event:
/// Json): Json` program is compiled, linked against both
/// thaw-arena and thaw-runtime, run as a real subprocess against a
/// mock Lambda Runtime API server (the same protocol thaw-runtime
/// itself is tested against), and its actual HTTP interaction is
/// verified end to end.
#[test]
fn compiles_and_runs_a_json_lambda_handler_against_a_mock_runtime_api() {
    let source = r#"
        function handler(event: Json): Json {
            console.log(String(event.message));
            return event;
        }
    "#;
    let (post_request, stdout) = compile_and_invoke_lambda(
        source,
        "json_lambda_handler",
        "{\"message\":\"ping\",\"ok\":true}",
    );

    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/response"));
    assert!(post_request.ends_with("{\"message\":\"ping\",\"ok\":true}"));
    // Also confirms the fflush-after-console.log fix: stdout is a pipe
    // here (fully buffered by default in libc), and the process is
    // killed rather than exited normally, so without an explicit flush
    // this assertion would flake/fail.
    assert!(stdout.contains("ping"));
}

#[test]
fn awaits_an_async_json_lambda_handler() {
    let source = r#"
        async function handler(event: Json): Promise<Json> {
            await sleep(1);
            return event;
        }
    "#;
    let (post_request, _) = compile_and_invoke_lambda(
        source,
        "async_json_lambda_handler",
        "{\"message\":\"after await\"}",
    );

    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/response"));
    assert!(post_request.ends_with("{\"message\":\"after await\"}"));
}

#[test]
fn posts_json_lambda_handler_failures_to_the_error_endpoint() {
    let source = r#"
        function handler(event: Json): Json {
            throw "json handler failed";
        }
    "#;
    let (post_request, _) =
        compile_and_invoke_lambda(source, "failing_json_lambda_handler", "{\"ok\":false}");

    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/error"));
    assert!(post_request.contains("json handler failed"));
}

#[test]
fn posts_async_json_lambda_rejections_to_the_error_endpoint() {
    let source = r#"
        async function handler(event: Json): Promise<Json> {
            await sleep(1);
            throw "async json handler failed";
        }
    "#;
    let (post_request, _) = compile_and_invoke_lambda(
        source,
        "rejecting_async_json_lambda_handler",
        "{\"ok\":false}",
    );

    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/error"));
    assert!(post_request.contains("async json handler failed"));
}
