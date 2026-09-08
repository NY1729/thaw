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

