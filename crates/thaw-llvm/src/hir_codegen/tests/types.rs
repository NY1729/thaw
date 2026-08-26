#[test]
fn compiles_multi_argument_generic_specializations_end_to_end() {
    let source = r#"
        interface Pair<T, U> {
            first: T;
            second: U;
        }

        function main(): void {
            console.log(chooseFirst(42, "unused"));
            console.log(chooseFirst("chosen", 7));
            console.log(chooseFirst(43, "deduplicated"));
            const pair = makePair("answer", 44);
            console.log(pair.first);
            console.log(pair.second);
            console.log(forward(45, "through generic"));
        }

        function chooseFirst<T, U>(first: T, second: U): T {
            return first;
        }

        function makePair<T, U>(first: T, second: U): Pair<T, U> {
            return { first: first, second: second };
        }

        function forward<T, U>(first: T, second: U): T {
            return chooseFirst(first, second);
        }
    "#;

    assert_eq!(
        compile_and_run(source, "multi_argument_generics"),
        "42\nchosen\n43\nanswer\n44\n45\n"
    );
}

#[test]
fn recursively_specializes_named_generic_local_function_values() {
    let source = r#"
        type Repeat = <T, U>(value: T, marker: U, remaining: number) => T;
        function main(): void {
            let calls = 0;
            const repeat: Repeat = function again<T, U>(
                value: T, marker: U, remaining: number
            ): T {
                calls += 1;
                if (remaining <= 0) { return value; }
                return again(value, marker, remaining - 1);
            };
            const alias = repeat;
            console.log(repeat("done", true, 2));
            console.log(calls);
            calls = 0;
            console.log(alias<number, string>(42, "marker", 3));
            console.log(calls);
            const normalize = function normalize<T>(
                value: T, index: number, values: T[]
            ): T {
                if (index <= 0) { return value; }
                return normalize(value, index - 1, values);
            };
            console.log(["a", "b", "c"].map(normalize).join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "recursive_named_generic_local_functions"),
        "done\n3\n42\n4\na,b,c\n"
    );
}

#[test]
fn compiles_forward_type_aliases_across_native_layouts() {
    let source = r#"
        type RecordValue = Named & { count: number };
        type Named = { name: string };
        type Choice = string | number;
        type ChoiceList = Choice[];
        type Label = string;
        interface AliasItem {
            label: Label;
            choices: ChoiceList;
        }
        function record(name: string, count: number): RecordValue {
            return { count, name };
        }
        function choose(flag: boolean): Choice {
            return flag ? "selected" : 8;
        }
        async function delayed(value: Choice): Promise<Choice> {
            await sleep(1);
            return value;
        }
        async function main(): Promise<void> {
            const item: RecordValue = record("alias", 3);
            console.log(item.name + String(item.count));
            const values: ChoiceList = [choose(true), choose(false)];
            console.log(values[0]);
            console.log(values[1]);
            console.log(await delayed("later"));
            const interfaceItem: AliasItem = {
                choices: ["first", 2],
                label: "interface"
            };
            console.log(interfaceItem.label);
            console.log(interfaceItem.choices[0]);
            console.log(interfaceItem.choices[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "forward_type_aliases"),
        "alias3\nselected\n8\nlater\ninterface\nfirst\n2\n"
    );
}

#[test]
fn compiles_generic_type_alias_instantiations() {
    let source = r#"
        type Boxed<T> = { value: T };
        type Wrapped<T> = Boxed<T>;
        type List<T> = T[];
        type Maybe<T> = T | undefined;
        type Mapper<T, U> = (value: T) => U;
        function first<T>(boxed: Wrapped<T>): T { return boxed.value; }
        function maybe(flag: boolean): Maybe<number> {
            return flag ? 5 : undefined;
        }
        function apply(callback: Mapper<number, string>): string {
            return callback(4);
        }
        function main(): void {
            console.log(first({ value: 3 }));
            console.log(first({ value: "generic" }));
            const values: List<string> = ["a", "b"];
            console.log(values[1]);
            console.log(maybe(true));
            console.log(maybe(false));
            console.log(apply((value: number): string => "value=" + String(value)));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_type_aliases"),
        "3\ngeneric\nb\n5\nundefined\nvalue=4\n"
    );
}

#[test]
fn compiles_constrained_generic_functions() {
    let source = r#"
        function numeric<T extends number>(value: T): T { return value; }
        function scalar<T extends number | string>(value: T): T { return value; }
        function named<T extends { name: string }>(value: T): T { return value; }
        function choose<T, U extends T>(left: T, right: U): U { return right; }
        function main(): void {
            console.log(numeric(6));
            console.log(scalar("value"));
            console.log(named({ name: "object", count: 2 }).name);
            console.log(choose("left", "right"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "constrained_generic_functions"),
        "6\nvalue\nobject\nright\n"
    );
}

#[test]
fn compiles_generic_function_type_defaults() {
    let source = r#"
        function make<T = string>(): T { return "ready"; }
        function duplicate<T, U = T>(value: T): { first: T; second: U } {
            return { first: value, second: value };
        }
        function numeric<T extends number = number>(): T { return 9; }
        function main(): void {
            console.log(make());
            const pair = duplicate("same");
            console.log(pair.first + pair.second);
            console.log(numeric());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_function_defaults"),
        "ready\nsamesame\n9\n"
    );
}

#[test]
fn compiles_explicit_generic_function_type_arguments() {
    let source = r#"
        function identity<T>(value: T): T { return value; }
        function pair<T, U = string>(left: T, right: U): { left: T; right: U } {
            return { left, right };
        }
        function numeric<T extends number = number>(): T { return 10; }
        function empty<T>(): T[] { return []; }
        function main(): void {
            console.log(identity<string>("explicit"));
            console.log(identity<number>(7));
            const value = pair<number>(4, "default");
            console.log(value.left);
            console.log(value.right);
            console.log(numeric<number>());
            console.log(empty<number>().length);
            console.log(empty<string>().length);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "explicit_generic_function_types"),
        "explicit\n7\n4\ndefault\n10\n0\n0\n"
    );
}

#[test]
fn compiles_contextually_specialized_generic_callbacks() {
    let source = r#"
        function identity<T>(value: T): T { return value; }
        async function main(): Promise<void> {
            const numbers = [1, 2].map(identity);
            const strings = ["a", "b"].map(identity);
            console.log(numbers[1]);
            console.log(strings[0]);
            const promised: number = await new Promise<number>((resolve, reject) => {
                resolve(3);
            }).then(identity);
            console.log(promised);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_callback_specialization"),
        "2\na\n3\n"
    );
}

#[test]
fn compiles_contextual_generic_arrow_callbacks() {
    let source = r#"
        async function main(): Promise<void> {
            const numbers = [3, 4].map(<T>(value: T): T => value);
            const strings = ["x", "y"].map(<T extends string>(value: T): T => value);
            console.log(numbers[0]);
            console.log(strings[1]);
            const promised: number = await new Promise<number>((resolve, reject) => {
                resolve(5);
            }).then(<T extends number>(value: T): T => value);
            console.log(promised);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "contextual_generic_arrows"),
        "3\ny\n5\n"
    );
}

#[test]
fn compiles_generic_instantiation_function_values() {
    let source = r#"
        function identity<T>(value: T): T { return value; }
        function pair<T, U = string>(left: T, right: U): { left: T; right: U } {
            return { left, right };
        }
        function main(): void {
            const numberIdentity: (value: number) => number = identity<number>;
            const stringIdentity: (value: string) => string = identity<string>;
            console.log(numberIdentity(11));
            console.log(stringIdentity("instantiated"));
            const makePair: (left: number, right: string) => { left: number; right: string } =
                pair<number>;
            const value = makePair(2, "pair");
            console.log(value.left);
            console.log(value.right);
            console.log([6, 7].map(numberIdentity)[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_instantiation_values"),
        "11\ninstantiated\n2\npair\n7\n"
    );
}

#[test]
fn compiles_annotated_contextual_generic_arrows() {
    let source = r#"
        function main(): void {
            const numberIdentity: (value: number) => number =
                <T>(value: T): T => value;
            const stringIdentity: (value: string) => string =
                <T extends string>(value: T): T => value;
            const choose: (left: number, right: string) => number =
                <T, U>(left: T, right: U): T => left;
            const inferredParameter: (value: number) => number = value => value + 1;
            console.log(numberIdentity(13));
            console.log(stringIdentity("context"));
            console.log(choose(8, "ignored"));
            console.log(inferredParameter(4));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "annotated_generic_arrows"),
        "13\ncontext\n8\n5\n"
    );
}

#[test]
fn compiles_local_generic_arrow_specializations() {
    let source = r#"
        async function main(): Promise<void> {
            const identity = <T>(value: T): T => value;
            const choose = <T, U>(left: T, right: U): T => left;
            const prefix: string = "value=";
            const describe = <T>(value: T): string => prefix + String(value);
            console.log(typeof identity);
            console.log(identity(17));
            console.log(identity("local"));
            console.log(identity(18));
            console.log(identity<number>(19));
            console.log(choose("selected", 4));
            const tuple: [string, number] = ["spread", 9];
            console.log(choose(...tuple));
            console.log(choose(...["literal", 2]));
            console.log(describe(5));
            console.log(describe(true));
            console.log([20, 21].map(identity)[1]);
            const promised: string = await new Promise<string>((resolve, reject) => {
                resolve("promise-arrow");
            }).then(identity);
            console.log(promised);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "local_generic_arrows"),
        "function\n17\nlocal\n18\n19\nselected\nspread\nliteral\nvalue=5\nvalue=true\n21\npromise-arrow\n"
    );
}

#[test]
fn compiles_generic_callbacks_for_user_functions() {
    let source = r#"
        function apply(callback: (value: number) => number, value: number): number {
            return callback(value);
        }
        function identity<T>(value: T): T { return value; }
        function main(): void {
            const local = <T>(value: T): T => value;
            console.log(apply(identity, 22));
            console.log(apply(local, 23));
            console.log(apply(<T extends number>(value: T): T => value + 1, 23));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_user_callbacks"),
        "22\n23\n24\n"
    );
}

#[test]
fn compiles_generic_function_type_alias_arrows() {
    let source = r#"
        type Identity = <T>(value: T) => T;
        type ForwardIdentity = IdentityChain;
        type IdentityChain = Identity;
        type LiteralIdentity = { <T>(value: T): T };
        type ParenthesizedIdentity = (<T>(value: T) => T);
        type ParenthesizedForward = (ParenthesizedIdentity);
        type Choose = <T, U>(left: T, right: U) => T;
        type Numeric = <T extends number>(value: T) => T;
        type DefaultFactory = <T = string>() => T;
        type AsyncIdentity = <T>(value: T) => Promise<T>;
        type Stringify = <T>(value: T) => string;
        type Nullify = <T>(value: T) => null;
        type Undefine = <T>(value: T) => undefined;
        type Predicate = <T>(value: T) => boolean;
        type NumericValue = <T>(value: T) => number;
        type NumericOperation = <T extends number>(value: T) => number;
        type FlaggedPredicate = <T>(value: T, flag: boolean) => boolean;
        type OptionalIdentity = <T>(value?: T) => T;
        function namedIdentity<T>(value: T): T { return value; }
        async function namedAsyncIdentity<T>(value: T): Promise<T> { return value; }
        async function main(): Promise<void> {
            const identity: Identity = <Value>(value: Value): Value => value;
            const choose: Choose = <Left, Right>(left: Left, right: Right): Left => left;
            const numeric: Numeric = <Value extends number>(value: Value): Value => value;
            const forwarded: Identity = namedIdentity;
            const chained: Identity = identity;
            const namedChain: Identity = forwarded;
            const aliasChain: ForwardIdentity = identity;
            const inferredArrowChain = aliasChain;
            const inferredNamedChain = namedChain;
            const literal: LiteralIdentity = <T>(value: T): T => value;
            const parenthesized: (ParenthesizedForward) = literal;
            const expression: Identity = function<T>(value: T): T { return value; };
            const inferredExpression = function<T>(value: T): T { return value; };
            const namedExpression: Identity = function localIdentity<T>(value: T): T { return value; };
            const factory: DefaultFactory = <Value = string>(): Value => "default";
            const inferredReturn: Identity = <T>(value: T) => value;
            const inferredFunctionReturn: Identity = function<T>(value: T) { return value; };
            const conditionalReturn: Identity = <T>(value: T) => true ? value : value;
            const branchedReturn: Identity = function<T>(value: T) { if (true) { return value; } return value; };
            const literalReturn: Stringify = <T>(value: T) => "constant";
            const nullReturn: Nullify = <T>(value: T) => null;
            const undefinedReturn: Undefine = <T>(value: T) => undefined;
            const equalsSelf: Predicate = <T>(value: T) => value === value;
            const stringifyValue: Stringify = <T>(value: T) => String(value);
            const numericValue: NumericValue = <T>(value: T) => Number(value);
            const decrement: NumericOperation = <T extends number>(value: T) => value - 1;
            const prefixed: Stringify = <T>(value: T) => "value=" + String(value);
            const logicalFlag: FlaggedPredicate = <T>(value: T, flag: boolean) => flag && true;
            const optionalIdentity: OptionalIdentity = <T>(value?: T): T => value;
            const asyncIdentity: AsyncIdentity = namedAsyncIdentity;
            console.log(identity(24));
            console.log(identity("alias"));
            console.log(choose("first", true));
            console.log(numeric(25));
            console.log(typeof forwarded);
            console.log(forwarded(26));
            console.log(forwarded<string>("forwarded"));
            console.log([27, 28].map(forwarded)[0]);
            console.log(chained<string>("arrow-chain"));
            console.log(namedChain(29));
            console.log(aliasChain<string>("type-alias-chain"));
            console.log(inferredArrowChain(30));
            console.log(inferredNamedChain<string>("inferred-chain"));
            console.log(literal<string>("call-literal-alias"));
            console.log(parenthesized(31));
            console.log(expression<string>("function-expression"));
            console.log(inferredExpression(32));
            console.log([33, 34].map(function<T>(value: T): T { return value; })[1]);
            console.log(namedExpression<string>("named-expression"));
            console.log(factory());
            console.log(inferredReturn<string>("inferred-return"));
            console.log(inferredFunctionReturn(35));
            console.log(await asyncIdentity<string>("async-alias"));
            console.log(conditionalReturn(36));
            console.log(branchedReturn<string>("branched-return"));
            console.log(literalReturn(true));
            console.log(nullReturn(37));
            console.log(undefinedReturn("ignored"));
            console.log(equalsSelf("same"));
            console.log(stringifyValue(38));
            console.log(numericValue("39"));
            console.log(decrement(41));
            console.log(prefixed(true));
            console.log(logicalFlag("ignored", true));
            console.log(optionalIdentity(41));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_function_alias_arrows"),
        "24\nalias\nfirst\n25\nfunction\n26\nforwarded\n27\narrow-chain\n29\ntype-alias-chain\n30\ninferred-chain\ncall-literal-alias\n31\nfunction-expression\n32\n34\nnamed-expression\ndefault\ninferred-return\n35\nasync-alias\n36\nbranched-return\nconstant\nnull\nundefined\ntrue\n38\n39\n40\nvalue=true\ntrue\n41\n"
    );
}

#[test]
fn compiles_generic_callable_interfaces() {
    let source = r#"
        interface Identity { <T>(value: T): T; }
        interface DerivedIdentity extends MiddleIdentity {}
        interface MiddleIdentity extends Identity {}
        type ForwardIdentity = IdentityChain;
        type IdentityChain = Identity;
        interface Numeric { <T extends number>(value: T): T; }
        function named<T>(value: T): T { return value; }
        function main(): void {
            const local: Identity = <Value>(value: Value): Value => value;
            const forwarded: Identity = named;
            const arrowChain: Identity = local;
            const namedChain: Identity = forwarded;
            const interfaceAliasChain: ForwardIdentity = local;
            const inherited: DerivedIdentity = forwarded;
            const numeric: Numeric = <Value extends number>(value: Value): Value => value;
            console.log(local(29));
            console.log(local("interface"));
            console.log(forwarded(30));
            console.log(forwarded<string>("named-interface"));
            console.log(numeric(31));
            console.log([32, 33].map(forwarded)[1]);
            console.log(arrowChain<string>("interface-chain"));
            console.log(namedChain(34));
            console.log(interfaceAliasChain<string>("interface-alias-chain"));
            console.log(inherited(35));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_callable_interfaces"),
        "29\ninterface\n30\nnamed-interface\n31\n33\ninterface-chain\n34\ninterface-alias-chain\n35\n"
    );
}

#[test]
fn compiles_generic_alias_defaults_and_constraints() {
    let source = r#"
        type Outcome<T, E = string> = { value: T; error: E };
        type SamePair<T, U = T> = { first: T; second: U };
        type Numeric<T extends number> = { value: T };
        function outcomeValue<T>(result: Outcome<T>): T { return result.value; }
        function outcome(value: number): Outcome<number> {
            return { error: "none", value };
        }
        function pair(value: string): SamePair<string> {
            return { second: value, first: value };
        }
        function numeric(value: number): Numeric<number> { return { value }; }
        function main(): void {
            const result: Outcome<number> = outcome(4);
            console.log(outcomeValue(result));
            console.log(result.error);
            const values: SamePair<string> = pair("same");
            console.log(values.first + values.second);
            console.log(numeric(7).value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_alias_defaults_constraints"),
        "4\nnone\nsamesame\n7\n"
    );
}

#[test]
fn compiles_generic_interface_defaults_and_constraints() {
    let source = r#"
        interface Outcome<T, E = string> { value: T; error: E }
        interface SamePair<T, U = T> { first: T; second: U }
        interface Numeric<T extends number> { value: T }
        function outcomeValue<T>(result: Outcome<T>): T { return result.value; }
        function outcome(value: number): Outcome<number> {
            return { error: "none", value };
        }
        function pair(value: string): SamePair<string> {
            return { second: value, first: value };
        }
        function numeric(value: number): Numeric<number> { return { value }; }
        function main(): void {
            const result: Outcome<number> = outcome(5);
            console.log(outcomeValue(result));
            console.log(result.error);
            const values: SamePair<string> = pair("pair");
            console.log(values.first + values.second);
            console.log(numeric(8).value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_interface_defaults_constraints"),
        "5\nnone\npairpair\n8\n"
    );
}

#[test]
fn compiles_generic_interface_inheritance() {
    let source = r#"
        interface Named { name: string }
        interface Box<T> { value: T }
        interface NamedBox<T> extends Named, Box<T> { count: number }
        interface Wrapped<T> extends NamedBox<T[]> { active: boolean }
        function first<T>(box: NamedBox<T>): T { return box.value; }
        function main(): void {
            const box: NamedBox<number> = { name: "items", value: 7, count: 1 };
            console.log(box.name);
            console.log(first(box));
            const wrapped: Wrapped<string> = {
                name: "wrapped", value: ["a", "b"], count: 2, active: true
            };
            console.log(wrapped.value[1]);
            console.log(wrapped.count);
            console.log(wrapped.active);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_interface_inheritance"),
        "items\n7\nb\n2\ntrue\n"
    );
}

#[test]
fn compiles_concrete_interface_extending_generic_base() {
    let source = r#"
        interface NumberEntry extends Entry<number> { enabled: boolean }
        interface Holder { entry: Entry<number> }
        interface TextPair extends Pair<string> { label: string }
        interface PairHolder { pair: Pair<number> }
        interface MetadataBox extends BoxWithMeta<number> {}
        interface Root { name: string }
        interface Entry<T> extends Root { value: T }
        interface DefaultEntry extends Entry<string> { count: number }
        type Pair<T> = { first: T; second: T };
        interface BoxWithMeta<T> { meta: Metadata; value: T }
        interface Metadata { tag: string }
        function main(): void {
            const number: NumberEntry = { name: "number", value: 12, enabled: true };
            console.log(number.name);
            console.log(number.value);
            console.log(number.enabled);
            const text: DefaultEntry = { name: "text", value: "ready", count: 3 };
            console.log(text.value);
            console.log(text.count);
            const holder: Holder = { entry: { name: "nested", value: 21 } };
            console.log(holder.entry.name);
            console.log(holder.entry.value);
            const pair: TextPair = { first: "a", second: "b", label: "pair" };
            console.log(pair.first + pair.second);
            const pairHolder: PairHolder = { pair: { first: 4, second: 5 } };
            console.log(pairHolder.pair.second);
            const metadata: MetadataBox = { meta: { tag: "forward" }, value: 8 };
            console.log(metadata.meta.tag);
            console.log(metadata.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "concrete_generic_base"),
        "number\n12\ntrue\nready\n3\nnested\n21\nab\n5\nforward\n8\n"
    );
}

#[test]
fn compiles_satisfies_and_non_null_assertions() {
    let source = r#"
        function main(): void {
            const config = { label: "ready", port: 40 } satisfies {
                label: string;
                port: number;
            };
            console.log(config.label);
            console.log(config.port + 2);
            const optional: number | undefined = 40;
            const nullable: number | null = 41;
            const nullish: number | null | undefined = 42;
            console.log(optional! + 2);
            console.log(nullable! + 1);
            console.log(nullish!);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "satisfies_and_non_null"),
        "ready\n42\n42\n42\n42\n"
    );
}

#[test]
fn compiles_readonly_array_and_tuple_types() {
    let source = r#"
        function first<T>(values: readonly T[]): T {
            return values[0];
        }
        function second(values: ReadonlyArray<number>): number {
            return values[1];
        }
        function main(): void {
            const tuple: readonly [number, string] = [40, "ready"];
            const values: ReadonlyArray<number> = [41, 42];
            console.log(tuple[0] + 2);
            console.log(tuple[1]);
            console.log(first<number>(values) + 1);
            console.log(second(values));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "readonly_arrays_and_tuples"),
        "42\nready\n42\n42\n"
    );
}

#[test]
fn compiles_partial_and_required_utility_types() {
    let source = r#"
        type Model = { value: number; label: string };
        type Patch = Partial<Model>;
        type CompletePatch = Required<Patch>;
        type GenericPatch<T> = Partial<{ value: T; label: string }>;
        function main(): void {
            const empty: Patch = {};
            const patch: GenericPatch<number> = { value: 42 };
            const complete: CompletePatch = { value: 42, label: "ready" };
            console.log(patch.value ?? 0);
            console.log(complete.label);
            console.log(empty.label ?? "empty");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "partial_and_required_utility_types"),
        "42\nready\nempty\n"
    );
}

#[test]
fn compiles_finite_record_utility_types() {
    let source = r#"
        type Counts = Record<"left" | "right", number>;
        type Pair<T> = Record<"first" | "second", T>;
        function total(values: Counts): number {
            return values.left + values.right;
        }
        function main(): void {
            const counts: Counts = { right: 22, left: 20 };
            const pair: Pair<string> = { first: "ready", second: "done" };
            console.log(total(counts));
            console.log(pair.first);
            console.log(pair.second);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "finite_record_utility_types"),
        "42\nready\ndone\n"
    );
}

#[test]
fn compiles_typed_string_index_signatures() {
    let source = r#"
        type Scores = { [key: string]: number; total: number };
        interface Labels<T> { [key: string]: T; primary: T }

        function main(): void {
            const scores: Scores = { alpha: 20, total: 20 };
            const key = "alpha";
            scores[key] += 1;
            scores.total++;

            const labels: Labels<string> = { primary: "re", other: "ady" };
            labels.other = "ady";
            console.log(scores[key] + scores.total);
            console.log(labels.primary + labels["other"]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "typed_string_index_signatures"),
        "42\nready\n"
    );
}

#[test]
fn compiles_pick_and_omit_utility_types() {
    let source = r#"
        type Model = { id: number; label: string; active: boolean };
        type Identity = Pick<Model, "id" | "label">;
        type WithoutId = Omit<Model, "id">;
        type GenericIdentity<T> = Pick<{ id: number; value: T; ignored: boolean }, "id" | "value">;
        function main(): void {
            const identity: Identity = { label: "ready", id: 42 };
            const rest: WithoutId = { active: true, label: identity.label };
            const generic: GenericIdentity<string> = { value: rest.label, id: identity.id };
            console.log(generic.id);
            console.log(generic.value);
            console.log(rest.active);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "pick_and_omit_utility_types"),
        "42\nready\ntrue\n"
    );
}

#[test]
fn compiles_optional_object_and_interface_properties() {
    let source = r#"
        interface Config { label: string; retries?: number }
        interface Box<T> { value?: T }
        type Inline = { note?: string };
        type Complete = Required<Config>;
        function main(): void {
            const config: Config = { label: "ready" };
            const box: Box<number> = {};
            const inline: Inline = {};
            const complete: Complete = { label: config.label, retries: 42 };
            console.log(config.retries ?? 0);
            console.log(box.value ?? complete.retries);
            console.log(inline.note ?? complete.label);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_object_and_interface_properties"),
        "0\n42\nready\n"
    );
}

#[test]
fn compiles_keyof_for_fixed_object_types() {
    let source = r#"
        interface Model { id: number; label: string }
        type ModelKey = keyof Model;
        type Clone = Pick<Model, keyof Model>;
        type Flags = Record<keyof Model, boolean>;
        type GenericClone<T> = Pick<T, keyof T>;
        function show(key: ModelKey): string { return key; }
        function main(): void {
            const clone: Clone = { label: "ready", id: 42 };
            const flags: Flags = { id: true, label: false };
            const generic: GenericClone<Model> = clone;
            console.log(show("label"));
            console.log(generic.id);
            console.log(flags.id);
            console.log(flags.label);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "keyof_fixed_object_types"),
        "label\n42\ntrue\nfalse\n"
    );
}

#[test]
fn compiles_fixed_object_indexed_access_types() {
    let source = r#"
        interface Model { id: number; label: string }
        type Label = Model["label"];
        type Value = Model[keyof Model];
        type GenericValue<T> = T["value"];
        function read(model: Model): Label { return model.label; }
        function main(): void {
            const model: Model = { id: 42, label: "ready" };
            const label: Label = read(model);
            const value: Value = label;
            const generic: GenericValue<{ value: number }> = model.id;
            console.log(value);
            console.log(generic);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "fixed_object_indexed_access_types"),
        "ready\n42\n"
    );
}

#[test]
fn compiles_interface_typed_object() {
    let source = r#"
        interface Point {
            x: number;
            y: number;
        }

        function dist(p: Point): number {
            return p.x + p.y;
        }

        function main(): void {
            const p: Point = { y: 2, x: 1 };
            console.log(dist(p));
            p.x = p.x + 10;
            console.log(p.x);
        }
    "#;
    assert_eq!(compile_and_run(source, "interfaces"), "3\n11\n");
}

#[test]
fn compiles_interface_extends() {
    let source = r#"
        interface Shape {
            color: number;
        }
        interface Circle extends Shape {
            radius: number;
        }

        function area(c: Circle): number {
            return c.radius * c.radius;
        }

        function main(): void {
            const c: Circle = { radius: 3, color: 1 };
            console.log(c.color);
            console.log(area(c));
        }
    "#;
    assert_eq!(compile_and_run(source, "interface_extends"), "1\n9\n");
}

#[test]
fn compiles_generic_interface_instantiation() {
    let source = r#"
        interface Box<T> {
            value: T;
        }

        function unwrapNumber(b: Box<number>): number {
            return b.value;
        }

        function main(): void {
            const b: Box<number> = { value: 42 };
            console.log(unwrapNumber(b));

            const s: Box<string> = { value: "hi" };
            console.log(s.value);
        }
    "#;
    assert_eq!(compile_and_run(source, "generic_interfaces"), "42\nhi\n");
}
