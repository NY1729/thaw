#[test]
fn compiles_nullable_undefined_and_tuple_callable_defaults() {
    let source = r#"
        function main(): void {
            const nullable = (x: number | null = null): number => x ?? 7;
            const tuple = (x: [number] = [7]): number => x[0];
            const readonlyTuple = (x: readonly [number] = [7]): number => x[0];
            const destructured = ([x]: [number] = [7]): number => x;
            const nullableBound = nullable.bind(null);
            const tupleBound = tuple.bind(null);
            const readonlyBound = readonlyTuple.bind(null);
            const destructuredBound = destructured.bind(null);
            console.log(nullable(3)); console.log(nullable(null));
            console.log(nullable()); console.log(nullable(undefined));
            console.log(nullableBound(3)); console.log(nullableBound(null));
            console.log(nullableBound()); console.log(nullableBound(undefined));
            console.log(tuple([3])); console.log(tuple()); console.log(tuple(undefined));
            console.log(tupleBound([3])); console.log(tupleBound()); console.log(tupleBound(undefined));
            console.log(readonlyTuple([3])); console.log(readonlyTuple()); console.log(readonlyTuple(undefined));
            console.log(readonlyBound([3])); console.log(readonlyBound()); console.log(readonlyBound(undefined));
            console.log(destructured([3])); console.log(destructured()); console.log(destructured(undefined));
            console.log(destructuredBound([3])); console.log(destructuredBound()); console.log(destructuredBound(undefined));
            const maybeUndefined = (x: number | undefined = undefined) => x;
            console.log(maybeUndefined(3)); console.log(maybeUndefined()); console.log(maybeUndefined(undefined));
            let calls = 0;
            const fallback = (): number => { calls += 1; return 9; };
            const lazy = (x: number | null = fallback()): number => x ?? 7;
            console.log(lazy(3)); console.log(calls);
            console.log(lazy(null)); console.log(calls);
            console.log(lazy()); console.log(calls);
            console.log(lazy(undefined)); console.log(calls);
            const narrowedUndefined = (x: number | undefined = 1) => x + 1;
            console.log(narrowedUndefined());
            const nullableFallback = (): number | null => null;
            const fallbackNullable = (x: number | null = nullableFallback()): number => x ?? 7;
            console.log(fallbackNullable()); console.log(fallbackNullable(3));
            const undefinedFallback = (): number | undefined => undefined;
            const fallbackUndefined = (x: number | undefined = undefinedFallback()) => x;
            console.log(fallbackUndefined()); console.log(fallbackUndefined(3));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nullable_undefined_tuple_callable_defaults"),
        "3\n7\n7\n7\n3\n7\n7\n7\n3\n7\n7\n3\n7\n7\n3\n7\n7\n3\n7\n7\n3\n7\n7\n3\n7\n7\n3\nundefined\nundefined\n3\n0\n7\n0\n9\n1\n9\n2\n2\n7\n3\nundefined\n3\n"
    );
}

#[test]
fn compiles_generator_callable_shapes_and_invocation_timing() {
    let source = r#"
        async function main(): Promise<void> {
            let calls = 0; let bodyRuns = 0;
            const values = function* (value?: number): Generator<number> { yield value ?? -1; };
            const alias = values;
            const bound = values.bind(null);
            console.log(alias().next().value); console.log(bound().next().value); console.log(bound(7).next().value);
            const contextual: (value?: number) => Generator<number> = function* (value: number = 7): Generator<number> { yield value; };
            console.log(contextual().next().value); console.log(contextual(3).next().value);
            const defaults = async function* (value: number = (calls += 1, 7)): AsyncGenerator<number> {
                bodyRuns += 1; yield value;
            };
            const deferred = defaults.bind(null);
            console.log(calls); const iterator = deferred(); console.log(calls, bodyRuns);
            console.log((await iterator.next()).value); console.log(calls, bodyRuns);
            console.log((await defaults(3).next()).value); console.log(calls, bodyRuns);
            const rest = async function* (...items: number[]): AsyncGenerator<number> { yield items.length; };
            const boundRest = rest.bind(null);
            console.log((await rest().next()).value); console.log((await rest(1, 2).next()).value);
            console.log((await boundRest().next()).value); console.log((await boundRest(1, 2).next()).value);
            const receiver = function* (this: { base: number }, value?: number): Generator<number> { yield this.base + (value ?? 0); };
            const holder = { base: 4 };
            const boundReceiver = receiver.bind(holder);
            console.log(receiver.call(holder).next().value); console.log(receiver.call(holder, 3).next().value);
            console.log(boundReceiver().next().value); console.log(boundReceiver(3).next().value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_callable_shapes_and_invocation_timing"),
        "-1\n-1\n7\n7\n3\n0\n1 0\n7\n1 1\n3\n1 2\n0\n2\n0\n2\n4\n7\n4\n7\n"
    );
}

#[test]
fn compiles_mixed_nullish_and_optional_tuple_callable_defaults() {
    let source = r#"
        function main(): void {
            const nullFallback = (x: number | null | undefined = null): string => x === null ? "null" : String(x);
            const scalarFallback = (x: number | null | undefined = 3): string => x === null ? "null" : String(x);
            const maybeFallback = (): number | undefined => undefined;
            const optionalFallback = (x: number | null | undefined = maybeFallback()): string => x === null ? "null" : String(x);
            const nullBound = nullFallback.bind(null);
            const scalarBound = scalarFallback.bind(null);
            const optionalBound = optionalFallback.bind(null);
            console.log(nullFallback()); console.log(nullFallback(undefined)); console.log(nullFallback(null)); console.log(nullFallback(5));
            console.log(nullBound()); console.log(nullBound(undefined)); console.log(nullBound(null)); console.log(nullBound(5));
            console.log(scalarFallback()); console.log(scalarFallback(undefined)); console.log(scalarFallback(null)); console.log(scalarFallback(5));
            console.log(scalarBound()); console.log(scalarBound(undefined)); console.log(scalarBound(null)); console.log(scalarBound(5));
            console.log(optionalFallback()); console.log(optionalFallback(undefined)); console.log(optionalFallback(null)); console.log(optionalFallback(5));
            console.log(optionalBound()); console.log(optionalBound(undefined)); console.log(optionalBound(null)); console.log(optionalBound(5));
            const tuple = (x: [number] | undefined = [7]): number => x[0];
            const readonlyTuple = (x: readonly [number] | undefined = [8]): number => x[0];
            const arrayPattern = ([x]: [number] | undefined = [9]): number => x;
            const tupleBound = tuple.bind(null); const readonlyBound = readonlyTuple.bind(null);
            const arrayPatternBound = arrayPattern.bind(null);
            console.log(tuple()); console.log(tuple(undefined)); console.log(tuple([4]));
            console.log(tupleBound()); console.log(tupleBound(undefined)); console.log(tupleBound([4]));
            console.log(readonlyTuple()); console.log(readonlyTuple(undefined)); console.log(readonlyTuple([6]));
            console.log(readonlyBound()); console.log(readonlyBound(undefined)); console.log(readonlyBound([6]));
            console.log(arrayPattern()); console.log(arrayPattern(undefined)); console.log(arrayPattern([10]));
            console.log(arrayPatternBound()); console.log(arrayPatternBound(undefined)); console.log(arrayPatternBound([10]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "mixed_nullish_optional_tuple_callable_defaults"),
        "null\nnull\nnull\n5\nnull\nnull\nnull\n5\n3\n3\nnull\n5\n3\n3\nnull\n5\nundefined\nundefined\nnull\n5\nundefined\nundefined\nnull\n5\n7\n7\n4\n7\n7\n4\n8\n8\n6\n8\n8\n6\n9\n9\n10\n9\n9\n10\n"
    );
}

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
fn named_generic_template_operations_runtime_matrix() {
    let source = r#"
        function identity<T>(value: T): T { return value; }
        function pair<T, U>(left: T, right: U): U { return right; }
        function main(): void {
            const alias = identity;
            console.log(identity(1));
            console.log(alias(2));
            const typed: <U>(value: U) => U = identity;
            console.log(typed("typed"));
            const fixed = identity<number>;
            console.log(fixed(3));
            const bound = identity.bind(null);
            console.log(bound("bound"));
            const leading = pair.bind(null, 4);
            console.log(leading("leading"));
            console.log(identity.call(null, 5));
            console.log(identity.apply(null, [6]));
            const explicitBound = (identity<number>).bind(null);
            console.log(explicitBound(7));
            console.log((identity<number>).call(null, 8));
            const aliasBound = alias.bind(null);
            console.log(aliasBound("alias-bound"));
            console.log(alias.call(null, "alias-call"));
            const alias2 = alias;
            console.log(alias2("alias-of-alias"));
            const kind: string = typeof alias2;
            console.log(kind);
            console.log(alias(9));
            console.log(alias("ten"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "named_generic_template_operations"),
        "1\n2\ntyped\n3\nbound\nleading\n5\n6\n7\n8\nalias-bound\nalias-call\nalias-of-alias\nfunction\n9\nten\n"
    );
}

#[test]
fn named_generic_template_call_apply_specialize_and_preserve_order() {
    let source = r#"
        function identity<T>(value: T): T { console.log("body"); return value; }
        function maybe<T = number>(value?: T): T | undefined { console.log("maybe-body"); return value; }
        function collect<T>(first: T, ...rest: T[]): T { console.log("rest-body"); return first; }
        async function asyncIdentity<T>(value: T): Promise<T> { console.log("async-body"); return value; }
        function container(): [number] { console.log("container"); return [2]; }
        async function main(): Promise<void> {
            const alias = identity;
            console.log(identity.call((console.log("this"), null), (console.log("arg"), 1)));
            console.log(alias.call(null, "string"));
            console.log(identity.apply(null, container(), (console.log("extra"), [99])));
            console.log(alias.apply(null, ["apply-string"]));
            console.log(identity.apply(null, [3]));
            const maybeArgs: [number] | undefined = [5];
            console.log(maybe.apply(null, maybeArgs, (console.log("nullable-extra"), [])));
            const noArgs: [number] | undefined = undefined;
            console.log(maybe.apply(null, noArgs));
            const nullArgs: [number] | null = null;
            console.log(maybe.apply(null, nullArgs));
            const nullishArgs: [number] | null | undefined = undefined;
            console.log(maybe.apply(null, nullishArgs));
            console.log(maybe.apply(null));
            console.log(maybe.apply(null, []));
            console.log(collect.apply(null, [7, 8]));
            console.log(await asyncIdentity.call(null, "async-call"));
            console.log(await asyncIdentity.apply(null, [4]));
            {
                const identity = (value: number): number => 99;
                console.log(alias("kept"));
                console.log(alias.call(null, "kept-call"));
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "named_generic_template_call_apply"),
        "this\narg\nbody\n1\nbody\nstring\ncontainer\nextra\nbody\n2\nbody\napply-string\nbody\n3\nnullable-extra\nmaybe-body\n5\nmaybe-body\nundefined\nmaybe-body\nundefined\nmaybe-body\nundefined\nmaybe-body\nundefined\nmaybe-body\nundefined\nrest-body\n7\nasync-body\nasync-call\nasync-body\n4\nbody\nkept\nbody\nkept-call\n"
    );
}

#[test]
fn named_generic_template_bind_specializes_and_captures_once() {
    let source = r#"
        function identity<T>(value: T): T { return value; }
        function pair<T, U>(left: T, right: U): U { return right; }
        function optional<T = number>(value?: T): T | undefined { return value; }
        function collect<T>(first: T, ...rest: T[]): T { return first; }
        function pick<T, K extends keyof T>(obj: T, key: K): T[K] { return obj[key]; }
        function main(): void {
            let effects = 0;
            const object = { value: 7 };
            const identityAlias = identity;
            const bound = identityAlias.bind((effects++, null), (effects++, object));
            const alias = bound;
            console.log(typeof bound);
            console.log(alias() === object);
            console.log(effects);
            const pairBound = pair.bind(null, 1);
            console.log(pairBound("number"));
            console.log(pairBound(true));
            const optionalBound = optional.bind(null);
            console.log(optionalBound());
            console.log(optionalBound("optional"));
            const restBound = collect.bind(null, "rest");
            console.log(restBound());
            console.log(restBound("ignored", "tail"));
            const key = "name";
            const picked = pick.bind(null, { name: "key-capture" }, key);
            console.log(picked());
            const unknownValue: unknown = 8;
            if (typeof unknownValue === "number") {
                const narrowed = identity.bind(null, unknownValue);
                console.log(narrowed());
            }
            {
                const identity = (value: number): number => 99;
                console.log(alias() === object);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "named_generic_template_bind"),
        "function\ntrue\n2\nnumber\ntrue\nundefined\noptional\nrest\nrest\nkey-capture\n8\ntrue\n"
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
fn console_log_prints_a_bigint_member_of_a_union() {
    let source = r#"
        function pick(flag: boolean): string | bigint { return flag ? "text" : 123n; }
        function main(): void {
            console.log(pick(true));
            console.log(pick(false));
            console.log(123n);
        }
    "#;
    assert_eq!(compile_and_run(source, "console_union_bigint"), "text\n123n\n123n\n");
}

// A caught native value (not an Object/Json) is a private descriptor; console.log prints a fixed
// label chosen by its category tag and never reads its contents.
#[test]
fn console_log_prints_a_fixed_label_for_each_caught_native_exception_tag() {
    let source = r#"
        function main(): void {
            try { throw new Map<string, number>(); } catch (error) { console.log(error); }
            try { throw new Set<number>(); } catch (error) { console.log(error); }
            try { throw new WeakMap<object, number>(); } catch (error) { console.log(error); }
            try { throw Symbol("s"); } catch (error) { console.log(error); }
            try { throw Promise.resolve(1); } catch (error) { console.log(error); }
            try { throw [1, 2]; } catch (error) { console.log(error); }
        }
    "#;
    let parsed = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&parsed).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "console_native_exception");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string().to_string();
    assert!(ir.contains("thaw_exception_native_tag"), "{ir}");
    assert!(ir.contains("switch i64"), "{ir}");
    for label in [
        "Symbol()", "[ <items unknown> ]", "<Buffer>", "Map { <items unknown> }",
        "WeakMap { <items unknown> }", "Set { <items unknown> }", "WeakSet { <items unknown> }",
        "[Function]", "Promise { <pending> }", "[native exception]",
    ] {
        assert!(ir.contains(label), "missing label {label:?}:\n{ir}");
    }
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
fn compiles_named_generic_template_aliases_and_alias_chains() {
    let source = r#"
        function identity<T>(value: T): T { return value; }
        function main(): void {
            const alias = identity;
            const chain = alias;
            console.log(chain(17));
            console.log(chain("alias"));
            console.log(typeof chain);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "named_generic_template_aliases"),
        "17\nalias\nfunction\n"
    );
}

#[test]
fn compiles_named_generic_templates_in_monomorphic_contexts() {
    let source = r#"
        function identity<T>(value: T): T { return value; }
        async function main(): Promise<void> {
            const numberIdentity: (value: number) => number = identity;
            const alias = identity;
            const stringIdentity: (value: string) => string = alias;
            console.log(numberIdentity(19));
            console.log(stringIdentity("context"));
            const promised: number = await new Promise<number>((resolve, reject) => {
                resolve(23);
            }).then(identity);
            const promisedAlias: string = await new Promise<string>((resolve, reject) => {
                resolve("promise-context");
            }).then(alias);
            console.log(promised);
            console.log(promisedAlias);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "named_generic_template_contexts"),
        "19\ncontext\n23\npromise-context\n"
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
fn generic_optional_arrow_preserves_present_and_absent_argument_values() {
    let source = r#"
        type OptionalIdentity = <T>(value?: T) => T | undefined;
        function main(): void {
            const identity: OptionalIdentity = <T>(value?: T): T | undefined => value;
            console.log(identity(41));
            console.log(identity<number>());
            const maybe: number | undefined = undefined;
            console.log(identity(maybe));
            const destructured: OptionalIdentity = <T>(value?: T): T | undefined => {
                [value] = [undefined];
                return value;
            };
            console.log(destructured(41));
            const iterated: OptionalIdentity = <T>(value?: T): T | undefined => {
                const values: (number | undefined)[] = [undefined];
                for (value of values) {}
                return value;
            };
            console.log(iterated(41));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_optional_arrow_values"),
        "41\nundefined\nundefined\nundefined\nundefined\n"
    );
}

#[test]
fn generic_optional_bound_arrow_accepts_explicit_type_arguments() {
    let source = r#"
        type OptionalIdentity = <T>(value?: T) => T | undefined;
        function main(): void {
            const identity: OptionalIdentity = <T>(value?: T): T | undefined => value;
            const later = identity.bind(null);
            console.log(later(41));
            console.log(later<number>(41));
            console.log(later<number>());
            console.log(later<string>("ok"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_optional_arrow_bind_explicit_types"),
        "41\n41\nundefined\nok\n"
    );
}

#[test]
fn bound_non_generic_optional_arrow_accepts_omitted_argument() {
    let source = r#"
        function main(): void {
            const plain = (value?: number): number | undefined => value;
            const later = plain.bind(null);
            console.log(later());
        }
    "#;
    assert_eq!(compile_and_run(source, "bound_non_generic_optional_arrow"), "undefined\n");
}

#[test]
fn bound_optional_rest_arrow_packs_call_time_values() {
    let source = r#"
        function main(): void {
            const f = (first?: number, ...rest: number[]): number => rest.length + (first ?? 0);
            const g = f.bind(null);
            console.log(g());
            console.log(g(3, 4, 5));
        }
    "#;
    assert_eq!(compile_and_run(source, "bound_optional_rest_arrow"), "0\n5\n");
}

#[test]
fn bound_named_optional_rest_and_fixed_functions_keep_call_shapes() {
    let source = r#"
        function optionalRest(first?: number, ...rest: number[]): number {
            return rest.length + (first ?? 0);
        }
        function add(left: number, right: number): number { return left + right; }
        function main(): void {
            const packed = optionalRest.bind(null);
            console.log(packed());
            console.log(packed(3, 4, 5));
            const addFive = add.bind(null, 5);
            console.log(addFive(2));
        }
    "#;
    assert_eq!(compile_and_run(source, "bound_named_optional_rest"), "0\n5\n7\n");
}

#[test]
fn named_callable_values_keep_optional_default_and_rest_shapes() {
    let source = r#"
        function optional(value?: number): number { return value ?? 8; }
        function other(value?: number): number { return value ?? 9; }
        function defaulted(value: number = 7): number { return value; }
        function packed(first?: number, ...rest: number[]): number { return rest.length + (first ?? 0); }
        function main(): void {
            const a = optional;
            const b = defaulted;
            const c = packed;
            const box = { optional };
            const list = [packed];
            const restBox = { packed };
            function factory() { return defaulted; }
            console.log(a(), a(2), a.call(null), a.apply(null, [2]));
            console.log(b(), b(3), box.optional(), factory()());
            console.log(c(), c(2, 3, 4), list[0](), c.call(null), c.apply(null, [2, 3]));
            console.log(restBox.packed());
            const aliasBound = b.bind(null);
            const packedBound = c.bind(null);
            console.log(aliasBound(), packedBound());
            const bound = defaulted.bind(null);
            console.log(bound());
            console.log(packed.bind(null)());
        }
    "#;
    assert_eq!(compile_and_run(source, "named_callable_value_shapes"), "8 2 8 2\n7 3 8 7\n0 4 0 0 3\n0\n7 0\n7\n0\n");
}

#[test]
fn named_default_parameters_apply_to_dynamic_undefined_but_preserve_null() {
    let source = r#"
        function defaulted(value: any = 7): any { return value; }
        function main(): void {
            const missing: any = undefined;
            const explicitNull: any = null;
            const alias: (value?: any) => any = defaulted;
            console.log(defaulted(undefined), defaulted(missing), defaulted(explicitNull));
            console.log(alias(missing), alias(explicitNull));
        }
    "#;
    assert_eq!(compile_and_run(source, "named_default_dynamic_undefined"), "7 7 null\n7 null\n");
}

#[test]
fn native_function_defaults_are_lazy_ordered_and_fresh() {
    let source = r#"
        let calls = "";
        function mark(value: number): number { calls += value + ","; return value; }
        function ordered(first: number = mark(1), required: number, later: number = mark(first + 1)): number {
            return required + later;
        }
        function fresh(value: {count: number} = {count: 0}): number { value.count += 1; return value.count; }
        function main(): void {
            console.log(ordered(undefined, 4), calls);
            console.log(ordered(3, 5, 6), calls);
            console.log(fresh(), fresh());
        }
    "#;
    assert_eq!(compile_and_run(source, "native_function_defaults_ordered_and_fresh"), "6 1,2,\n11 1,2,\n1 1\n");
}

#[test]
fn native_function_default_parameter_destructuring_reads_getter_once() {
    let source = r#"
        let reads = 0;
        function read({ nested: { value = 9 } }: { nested: { value?: number } }): number {
            return value;
        }
        function main(): void {
            const source = {
                get nested(): { value?: number } {
                    reads++;
                    return { value: undefined };
                },
            };
            console.log(read(source), reads);
        }
    "#;
    assert_eq!(compile_and_run(source, "native_parameter_destructuring_getter_once"), "9 1\n");
}

#[test]
fn named_callable_value_identity_and_nullable_default_are_preserved() {
    let source = r#"
        function defaulted(value: number = 5): number { return value; }
        function main(): void {
            const d1 = defaulted;
            const d2 = defaulted;
            const defaultBox = { defaulted };
            const defaultList = [defaulted];
            function factory() { return defaulted; }
            const exact: (value: number) => number = defaulted;
            console.log(exact(12));
            console.log(d1 === d2, d1 === defaulted, defaultBox.defaulted === defaulted,
                defaultList[0] === defaulted, factory() === defaulted, exact === defaulted);
        }
    "#;
    assert_eq!(compile_and_run(source, "named_callable_value_identity_nullable"), "12\ntrue true true true true true\n");
}

#[test]
fn required_nullable_named_callable_accepts_present_value() {
    let source = r#"
        function maybe(value: number | undefined): number { return value ?? 4; }
        function main(): void {
            const exact: (value: number) => number = maybe;
            console.log(exact(17));
        }
    "#;
    assert_eq!(compile_and_run(source, "required_nullable_named_callable"), "17\n");
}

#[test]
fn optional_callable_alias_identity_and_nullable_default_values_are_preserved() {
    let source = r#"
        function optional(value?: number): number { return value ?? 8; }
        function other(value?: number): number { return value ?? 9; }
        function main(): void {
            const a = optional;
            const b = optional;
            const box = { optional };
            const list = [optional];
            const required: (value: number | undefined) => number = optional;
            console.log(a === b, a === optional, box.optional === optional, list[0] === optional);
            console.log(required === optional);
            console.log(a !== other);
            console.log(a(), a(2));
            console.log(optional.bind(null) !== optional, optional.bind(null) !== optional.bind(null));
        }
    "#;
    assert_eq!(compile_and_run(source, "optional_callable_alias_identity_nullable"), "true true true true\ntrue\ntrue\n8 2\ntrue true\n");
}

#[test]
fn named_nullable_default_adapter_preserves_null_and_values() {
    let source = r#"
        function nullable(value: number | null = null): number { return value ?? 7; }
        function main(): void {
            const direct = nullable;
            const bound = nullable.bind(null);
            console.log(direct(), direct(null), direct(3), bound(), bound(null), bound(3));
            console.log(nullable.call(null), nullable.call(null, null), nullable.call(null, 3));
            console.log(nullable.apply(null, [null]), nullable.apply(null, [3]));
        }
    "#;
    assert_eq!(compile_and_run(source, "named_nullable_default_adapter"), "7 7 3 7 7 3\n7 7 3\n7 3\n");
}

#[test]
fn named_callable_any_projection_keeps_function_identity() {
    let source = r#"
        function optional(value?: number): number { return value ?? 8; }
        function main(): void {
            const dynamic: any = optional;
            console.log(dynamic === optional);
        }
    "#;
    assert_eq!(compile_and_run(source, "named_callable_any_identity"), "true\n");
}

#[test]
fn bound_named_optional_and_default_functions_accept_omitted_arguments() {
    let source = r#"
        function plain(value?: number): number | undefined { return value; }
        function defaulted(value: number = 7): number { return value; }
        function main(): void {
            const later = plain.bind(null);
            const laterDefault = defaulted.bind(null);
            console.log(later());
            console.log(laterDefault());
        }
    "#;
    assert_eq!(compile_and_run(source, "bound_named_omissions"), "undefined\n7\n");
}

#[test]
fn bound_default_parameters_evaluate_at_each_invocation() {
    let source = r#"
        let calls = 0;
        const f = (value: number = ++calls): number => value;
        const g = f.bind(null);
        function main(): void {
            console.log(calls);
            console.log(g());
            console.log(g());
            console.log(calls);
        }
    "#;
    assert_eq!(compile_and_run(source, "bound_default_invocation_timing"), "0\n1\n2\n2\n");
}

#[test]
fn bound_named_default_uses_omission_adapter_lazily() {
    let source = r#"
        let calls = 0;
        function defaulted(value: number = ++calls): number { return value; }
        function main(): void {
            const bound = defaulted.bind(null);
            console.log(calls);
            console.log(bound());
            console.log(bound());
            console.log(bound(9));
            console.log(calls);
        }
    "#;
    assert_eq!(compile_and_run(source, "bound_named_default_lazy"), "0\n1\n2\n9\n2\n");
}

#[test]
fn bound_leading_argument_effect_runs_once_before_repeated_calls() {
    let source = r#"
        let calls = 0;
        function next(): number { calls++; return 4; }
        function add(first: number, later?: number): number { return first + (later ?? 0); }
        function main(): void {
            const bound = add.bind(null, next());
            console.log(calls);
            console.log(bound());
            console.log(bound(3));
            console.log(calls);
        }
    "#;
    assert_eq!(compile_and_run(source, "bound_leading_argument_once"), "1\n4\n7\n1\n");
}

#[test]
fn required_callback_context_calls_defaulted_arrow_with_supplied_value() {
    let source = r#"
        function invoke(cb: (value: number) => number): number { return cb(9); }
        function main(): void {
            console.log(invoke((value: number = 5): number => value));
        }
    "#;
    assert_eq!(compile_and_run(source, "required_callback_default_control"), "9\n");
}

#[test]
fn required_callback_context_calls_defaulted_stored_arrow_and_function_expression() {
    let source = r#"
        function invoke(cb: (value: number) => number): number { return cb(9); }
        function main(): void {
            const local = (value: number = 5): number => value;
            const typed: (value: number) => number = (value: number = 5): number => value;
            console.log(invoke(local));
            console.log(invoke(typed));
            console.log(invoke(function(value: number = 5): number { return value; }));
        }
    "#;
    assert_eq!(compile_and_run(source, "required_callback_default_stored_and_fn"), "9\n9\n9\n");
}

#[test]
fn bound_optional_function_expression_keeps_this_receiver() {
    let source = r#"
        function main(): void {
            const f = function(this: { value: number }, arg?: number): number {
                return arg === undefined ? this.value : arg;
            };
            const g = f.bind({ value: 7 });
            console.log(g());
            console.log(g(3));
        }
    "#;
    assert_eq!(compile_and_run(source, "bound_optional_function_expression_this"), "7\n3\n");
}

#[test]
fn generic_arrow_bind_defers_specialization_until_direct_call() {
    let source = r#"
        function main(): void {
            const identity = <T>(value: T): T => value;
            const bound = identity.bind(null);
            console.log(bound(42));
        }
    "#;
    assert_eq!(compile_and_run(source, "generic_arrow_bind_deferred"), "42\n");
}

#[test]
fn generic_arrow_bind_combines_bound_and_call_site_inference() {
    let source = r#"
        function main(): void {
            const pair = <T, U>(left: T, right: U): U => right;
            const bound = pair.bind(null, 1);
            console.log(typeof bound);
            console.log(bound("x"));
        }
    "#;
    assert_eq!(compile_and_run(source, "generic_arrow_bind_mixed_inference"), "function\nx\n");
}

#[test]
fn generic_arrow_bind_evaluates_captures_once_at_binding_time() {
    let source = r#"
        let thisCount = 0;
        let argumentCount = 0;
        function nextThis(): null { thisCount++; return null; }
        function nextArgument(): number { argumentCount++; return 42; }
        function main(): void {
            const identity = <T>(value: T): T => value;
            const bound = identity.bind(nextThis(), nextArgument());
            console.log(thisCount);
            console.log(argumentCount);
        }
    "#;
    assert_eq!(compile_and_run(source, "generic_arrow_bind_capture_once"), "1\n1\n");
}

#[test]
fn generic_arrow_bind_aliases_specialize_independently_and_respect_shadowing() {
    let source = r#"
        function main(): void {
            const identity = <T>(value: T): T => value;
            const bound = identity.bind(null);
            const alias = bound;
            {
                const bound = (value: number): number => value + 1;
                console.log(bound(2));
            }
            console.log(alias(42));
            console.log(bound("ok"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_arrow_bind_alias_specializations"),
        "3\n42\nok\n"
    );
}

#[test]
fn generic_arrow_bind_keeps_lexical_this_but_evaluates_this_arg_once() {
    let source = r#"
        class Counter {
            value: number;
            constructor(value: number) { this.value = value; }
            make(): void {
                let effects = 0;
                function other(): Counter { effects++; return new Counter(99); }
                const read = <T>(unused: T): number => this.value;
                const bound = read.bind(other(), 0);
                console.log(bound());
                console.log(effects);
            }
        }
        function main(): void { new Counter(7).make(); }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_arrow_bind_lexical_this"),
        "7\n1\n"
    );
}

#[test]
fn generic_arrow_bind_preserves_omitted_optional_and_call_time_rest_values() {
    let source = r#"
        function main(): void {
            const optional = <T>(first: T, later?: T): T | undefined => later;
            const boundOptional = optional.bind(null, 42);
            console.log(boundOptional());
            const count = <T>(first: T, ...rest: T[]): number => rest.length;
            const boundRest = count.bind(null, 1);
            console.log(boundRest(2, 3));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_arrow_bind_optional_rest"),
        "undefined\n2\n"
    );
}

#[test]
fn generic_optional_native_rest_pads_fixed_slot_once() {
    let source = r#"
        function collect<T>(seed: T, value?: T, ...rest: T[]): number {
            return value === undefined ? 10 + rest.length : rest.length;
        }
        function main(): void {
            console.log(collect<number>(1));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_optional_native_rest_fixed_slot"),
        "10\n"
    );
}

#[test]
fn generic_arrow_bind_snapshots_tuple_spread_once() {
    let source = r#"
        function main(): void {
            const pair = <A, B>(left: A, right: B): string => `${left}:${right}`;
            const values: [number, string] = [1, "before"];
            const mutable: any = values;
            const bound = pair.bind(null, ...values);
            mutable[0] = 9;
            mutable[1] = "after";
            console.log(bound());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generic_arrow_bind_tuple_snapshot"),
        "1:before\n"
    );
}

#[test]
fn compiles_inline_generic_function_type_arrows() {
    let source = r#"
        function named<T>(value: T): T { return value; }
        function main(): void {
            const identity: <T>(value: T) => T = <U>(value: U): U => value;
            const choose: <T, U>(left: T, right: U) => T =
                <Left, Right>(left: Left, right: Right): Left => left;
            const forwarded: <T>(value: T) => T = named;
            console.log(identity(21));
            console.log(identity("inline"));
            console.log(choose(true, 22));
            console.log(forwarded("named"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "inline_generic_function_types"),
        "21\ninline\ntrue\nnamed\n"
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

#[test]
fn overloaded_bind_keeps_public_residuals_at_runtime() {
    let source = r#"
        function pick(prefix: number, value: number): number;
        function pick(prefix: string, value: string): string;
        function pick(prefix: number | string | boolean, value: number | string | boolean): number | string | boolean {
            if (typeof prefix === "number") return value;
            return value;
        }
        function main(): void {
            const numberBound = pick.bind(undefined, 1);
            const stringBound = pick.bind(undefined, "prefix");
            const numberCallback: (value: number) => number = numberBound;
            const stringCallback: (value: string) => string = stringBound;
            const repeated = numberBound.bind(undefined);
            console.log(numberBound(5));
            console.log(numberCallback(6));
            console.log(repeated(7));
            console.log(numberBound.call(undefined, 8));
            console.log(numberBound.apply(undefined, [9]));
            console.log(stringBound("value"));
            console.log(stringCallback("callback"));
        }
    "#;
    assert_eq!(compile_and_run(source, "overloaded_bind_residual_runtime"), "5\n6\n7\n8\n9\nvalue\ncallback\n");
}

#[test]
fn overloaded_function_callback_uses_selected_public_adapter_at_runtime() {
    let source = r#"
        function pick(value: number): number;
        function pick(value: string): string;
        function pick(value: number | string | boolean): number | string | boolean { return value; }
        function main(): void {
            const callback: (value: number) => number = pick;
            console.log(callback(9));
        }
    "#;
    assert_eq!(compile_and_run(source, "overloaded_callback_adapter_runtime"), "9\n");
}

#[test]
fn overloaded_bind_evaluates_receiver_and_prefix_once() {
    let source = r#"
        let receiverEffects = 0;
        let prefixEffects = 0;
        function pick(prefix: number, value: number): number;
        function pick(prefix: string, value: string): string;
        function pick(prefix: number | string | boolean, value: number | string | boolean): number | string | boolean { return value; }
        function nextReceiver(): number { receiverEffects += 1; return 0; }
        function nextPrefix(): number { prefixEffects += 1; return 1; }
        function main(): void {
            const bound = pick.bind(nextReceiver(), nextPrefix());
            console.log(bound(7));
            console.log(receiverEffects);
            console.log(prefixEffects);
        }
    "#;
    assert_eq!(compile_and_run(source, "overloaded_bind_once"), "7\n1\n1\n");
}

#[test]
fn overloaded_bind_retains_optional_and_rest_residuals_at_runtime() {
    let source = r#"
        function choose(prefix: string, value?: number): string;
        function choose(prefix: string, ...values: number[]): string;
        function choose(prefix: string | boolean, ...values: number[]): string | boolean { return prefix; }
        function main(): void {
            const bound = choose.bind(undefined, "p");
            console.log(bound());
            console.log(bound(1));
            console.log(bound(1, 2));
        }
    "#;
    assert_eq!(compile_and_run(source, "overloaded_bind_optional_rest_runtime"), "p\np\np\n");
}

#[test]
fn overloaded_bind_callback_context_adapts_physical_abi_at_runtime() {
    let source = r#"
        function useCallback(callback: (value: number) => string): string;
        function useCallback(callback: (value: string) => number): number;
        function useCallback(callback: (value: number | string) => string | number): string | number { return callback(1); }
        function main(): void {
            const bound = useCallback.bind(undefined, (value) => value.toFixed());
            console.log(bound());
        }
    "#;
    assert_eq!(compile_and_run(source, "overloaded_bind_callback_abi"), "1\n");
}
