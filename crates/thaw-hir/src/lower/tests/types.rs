#[test]
fn lowers_any_and_unknown_annotations_to_dynamic_values() {
    let program = lower(
        r#"
        function acceptUnknown(value: unknown): unknown { return value; }
        function acceptAny(value: any): any { return value; }
        function main(): void {}
        "#,
    );
    assert_eq!(program.functions[0].params[0].ty, HirType::Json);
    assert_eq!(program.functions[0].ret, HirType::Json);
    assert_eq!(program.functions[1].params[0].ty, HirType::Json);
    assert_eq!(program.functions[1].ret, HirType::Json);
}

#[test]
fn stringifies_an_unknown_value_through_the_dynamic_host() {
    lower(
        r#"
        function stringify(value: unknown): string {
            return JSON.stringify(value);
        }
        function main(): void {}
        "#,
    );
}

#[test]
fn infers_void_for_a_dynamic_callback_with_a_bare_return() {
    lower(
        r#"
        function main(): void {
            const target: JsValue = getDynamicValue("target");
            target.consume((value) => {
                if (value) { return; }
            });
        }
        "#,
    );
}

#[test]
fn contextual_callback_abi_overrides_explicit_any_parameters() {
    lower(
        r#"
        declare function subscribe(callback: (value: JsValue) => Promise<any>): void;
        function main(): void {
            subscribe(async (value: any) => value.child.read());
        }
        "#,
    );
}

#[test]
fn accepts_an_abi_compatible_object_prefix() {
    lower(
        r#"
        type Narrow = { first: number; last: string };
        type Wide = { first: number; last: string; middle: boolean };
        function accept(value: Narrow): string { return value.last; }
        function main(): void {
            const value: Wide = { first: 1, last: "ok", middle: true };
            console.log(accept(value));
        }
        "#,
    );
}

#[test]
fn callback_parameter_naming_a_subset_of_fields_in_any_order_is_accepted() {
    // Plain, async and generator callbacks whose annotation names only some of the supplied
    // fields (not a prefix, different order) are lowered at the supplied physical layout.
    lower(
        r#"
        declare function subscribe(callback: (r: { a: number; b: number; c: string }) => void): void;
        declare function subscribeLater(callback: (r: { a: number; b: number; c: string }) => Promise<void>): void;
        function main(): void {
            subscribe((r: { c: string; a: number }): void => { r.a = 7; });
            subscribeLater(async (r: { b: number }): Promise<void> => { r.b = 1; });
        }
        "#,
    );
}

#[test]
fn callback_parameter_naming_a_field_the_caller_does_not_supply_is_rejected() {
    let module = thaw_parser::parse_typescript(
        r#"
        declare function subscribe(callback: (r: { a: number; b: number }) => void): void;
        function main(): void {
            subscribe((r: { a: number; missing: number }): void => { r.a = 1; });
        }
        "#,
    )
    .unwrap();
    assert!(lower_module(&module).is_err());
}

#[test]
fn reports_a_missing_required_object_property() {
    let module = thaw_parser::parse_typescript(
        "type User = { name: string; age: number }; function main(): void { const user: User = { name: 'A' }; }",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert_eq!(error, "object literal is missing required property `age`");
}

#[test]
fn rejects_unsafe_union_array_composition() {
    let module = thaw_parser::parse_typescript(
        r#"
            function values(numbers: boolean): number[] | string[] {
                return numbers ? [1] : ["a"];
            }
            function main(): void { values(true).concat(2); }
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("not safely representable by every union receiver member"),
        "unexpected error: {error}"
    );

    let module = thaw_parser::parse_typescript(
        r#"
            function values(numbers: boolean): number[] | string[] {
                return numbers ? [1] : ["a"];
            }
            function main(): void { const map = new Map(values(true)); }
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("must contain [key, value] entries"),
        "unexpected error: {error}"
    );
}

#[test]
fn validates_satisfies_without_widening_the_expression() {
    let program = lower(
        r#"
            function main(): void {
                const value = { answer: 42 } satisfies { answer: number };
                console.log(value.answer);
            }
        "#,
    );
    assert!(matches!(
        program.functions[0].body[0],
        HirStmt::Let(_, HirType::Object(ref fields), _)
            if fields == &vec![("answer".into(), HirType::F64)]
    ));

    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const value = { answer: "wrong" } satisfies { answer: number };
        }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("expected F64"), "{error}");
}

#[test]
fn propagates_parameter_constraints_through_forward_call_chains() {
    let program = lower(
        "function first(value) { return second(value); } function second(value) { return value; } function main(): string { return first(\"ok\"); }",
    );
    assert_eq!(program.functions[0].params[0].ty, HirType::Str);
    assert_eq!(program.functions[0].ret, HirType::Str);
    assert_eq!(program.functions[1].params[0].ty, HirType::Str);
    assert_eq!(program.functions[1].ret, HirType::Str);
}

#[test]
fn rejects_conflicting_call_site_parameter_constraints() {
    let module = thaw_parser::parse_typescript(
        "function identity(value) { return value; } function main(): void { identity(1); identity(\"x\"); }",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("conflicting inferred types"),
        "unexpected error: {error}"
    );
    assert!(error.contains("at bytes"));
}

#[test]
fn monomorphizes_a_generic_function_from_its_call_site() {
    let program = lower(
        "function identity<T>(value: T): T { return value; } function main(): string { return identity(\"ok\"); }",
    );
    let identity = program
        .functions
        .iter()
        .find(|function| function.name == "identity__thaw_str")
        .unwrap();
    assert_eq!(identity.params[0].ty, HirType::Str);
    assert_eq!(identity.ret, HirType::Str);
}

#[test]
fn specializes_multiple_generic_arguments_as_one_call_tuple() {
    let program = lower(
        r#"
        interface Pair<T, U> { first: T; second: U; }
        function chooseFirst<T, U>(first: T, second: U): T { return first; }
        function makePair<T, U>(first: T, second: U): Pair<T, U> {
            return { first: first, second: second };
        }
        function main(): void {
            console.log(chooseFirst(1, "ignored"));
            const pair = makePair("left", 2);
            console.log(pair.first);
            console.log(pair.second);
        }
        "#,
    );
    let choose = program
        .functions
        .iter()
        .find(|function| function.name == "chooseFirst__thaw_f64__str")
        .expect("chooseFirst<number, string> specialization");
    assert_eq!(choose.params[0].ty, HirType::F64);
    assert_eq!(choose.params[1].ty, HirType::Str);
    assert_eq!(choose.ret, HirType::F64);

    let pair = program
        .functions
        .iter()
        .find(|function| function.name == "makePair__thaw_str__f64")
        .expect("makePair<string, number> specialization");
    assert_eq!(
        pair.ret,
        HirType::Object(vec![
            ("first".into(), HirType::Str),
            ("second".into(), HirType::F64),
        ])
    );
    // The returned literal is now wrapped in the native-owner staging; compare it unstaged.
    assert!(
        matches!(&pair.body[0], HirStmt::Return(Some(returned))
            if matches!(unstage_object_literal(returned, &[]), HirExpr::ObjectLit(fields)
                if fields[0].0 == "first" && fields[1].0 == "second")),
        "{:?}", pair.body[0]
    );
}

#[test]
fn enforces_repeated_generic_type_constraints_across_arguments() {
    let module = thaw_parser::parse_typescript(
        r#"
        function same<T>(left: T, right: T): T { return left; }
        function main(): void { same(1, "wrong"); }
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("conflicting call-site types"), "{error}");
    assert!(error.contains("F64") && error.contains("Str"), "{error}");
}

#[test]
fn enforces_declared_generic_function_constraints() {
    let primitive = thaw_parser::parse_typescript(
        r#"
        function numeric<T extends number>(value: T): T { return value; }
        function main(): void { numeric("wrong"); }
        "#,
    )
    .unwrap();
    let error = lower_module(&primitive).unwrap_err();
    assert!(error.contains("does not satisfy constraint F64"), "{error}");

    let dependent = thaw_parser::parse_typescript(
        r#"
        function choose<T, U extends T>(left: T, right: U): U { return right; }
        function main(): void { choose(1, "wrong"); }
        "#,
    )
    .unwrap();
    let error = lower_module(&dependent).unwrap_err();
    assert!(error.contains("does not satisfy constraint F64"), "{error}");

    let structural = thaw_parser::parse_typescript(
        r#"
        function named<T extends { name: string }>(value: T): T { return value; }
        function main(): void { named({ value: 1 }); }
        "#,
    )
    .unwrap();
    let error = lower_module(&structural).unwrap_err();
    assert!(
        error.contains("does not satisfy constraint Object"),
        "{error}"
    );
}

#[test]
fn validates_generic_function_defaults_against_constraints() {
    let module = thaw_parser::parse_typescript(
        r#"
        function invalid<T extends number = string>(): T { return "wrong"; }
        function main(): void { invalid(); }
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("does not satisfy constraint F64"), "{error}");
}

#[test]
fn rejects_required_type_parameters_after_defaults() {
    for (source, kind) in [
        (
            "interface Invalid<T = string, U> { first: T; second: U } function main(): void {}",
            "generic interface",
        ),
        (
            "type Invalid<T = string, U> = { first: T; second: U }; function main(): void {}",
            "generic type alias",
        ),
        (
            "function invalid<T = string, U>(value: U): U { return value; } function main(): void { invalid(1); }",
            "generic function",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(kind), "{error}");
        assert!(error.contains("required type parameter `U`"), "{error}");
    }
}

#[test]
fn validates_explicit_generic_function_type_arguments() {
    for (source, expected) in [
        (
            "function id<T>(value: T): T { return value; } function main(): void { id<string>(1); }",
            "explicit type is Str",
        ),
        (
            "function pair<T, U>(left: T, right: U): T { return left; } function main(): void { pair<number>(1, 2); }",
            "expects 2 explicit type argument",
        ),
        (
            "function numeric<T extends number>(value: T): T { return value; } function main(): void { numeric<string>(\"x\"); }",
            "does not satisfy constraint F64",
        ),
        (
            "function plain(value: number): number { return value; } function main(): void { plain<number>(1); }",
            "non-generic function `plain`",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn validates_contextually_specialized_generic_callbacks() {
    let module = thaw_parser::parse_typescript(
        r#"
        function numeric<T extends number>(value: T): T { return value; }
        function main(): void { ["wrong"].map(numeric); }
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("cannot specialize generic callback `numeric`"),
        "{error}"
    );
    assert!(error.contains("does not satisfy constraint F64"), "{error}");
}

#[test]
fn infers_generic_function_types_from_contextual_callbacks() {
    lower(
        r#"
        function transform<T, U>(value: T, callback: (value: T) => U): U {
            return callback(value);
        }
        function main(): void {
            const result: string = transform(
                21,
                value => String(value * 2),
            );
            console.log(result);
        }
        "#,
    );
}

#[test]
fn dispatches_local_function_overloads_by_argument_type() {
    lower(
        r#"
        function __thawmod0_label(value: number): string;
        function __thawmod0_label(value: string): string;
        function __thawmod0_label(value: number | string): string {
            return typeof value === "number" ? "n:" + String(value) : "s:" + value;
        }
        function main(): void {
            console.log(__thawmod0_label(4), __thawmod0_label("x"));
        }
        "#,
    );
}

#[test]
fn specializes_generic_keyof_indexed_accesses() {
    lower(
        r#"
        function read<T, K extends keyof T>(value: T, key: K): T[K] {
            return value[key];
        }
        function main(): void {
            const user = { name: "Alice", age: 21 };
            console.log(read(user, "name"), read(user, "age"));
        }
        "#,
    );
}

#[test]
fn preserves_union_narrowing_after_an_early_return() {
    lower(
        r#"
        type Value = { text: string } | { count: number };
        function show(value: Value): string {
            if ("text" in value && typeof value.text === "string") {
                return value.text;
            }
            return String(value.count);
        }
        function main(): void {
            console.log(show({ text: "ok" }), show({ count: 42 }));
        }
        "#,
    );
}

#[test]
fn narrows_unions_with_user_defined_type_predicates() {
    lower(
        r#"
        type Value = string | number;
        function isString(value: Value): value is string {
            return typeof value === "string";
        }
        function show(value: Value): string {
            return isString(value) ? value.toUpperCase() : String(value * 2);
        }
        function main(): void {
            console.log(show("ok"), show(21));
        }
        "#,
    );
}

#[test]
fn specializes_generic_type_predicates() {
    lower(
        r#"
        function present<T>(value: T | undefined): value is T {
            return value !== undefined;
        }
        function show(value: string | undefined): string {
            return present(value) ? value.toUpperCase() : "none";
        }
        function main(): void {
            console.log(show("ok"), show(undefined));
        }
        "#,
    );
}

#[test]
fn accepts_omitted_optional_tuple_elements() {
    lower(
        r#"
        function show(value: [string, number?]): string {
            const [name, count = 0] = value;
            return name + ":" + String(count);
        }
        function main(): void {
            console.log(show(["a"]), show(["b", 2]));
        }
        "#,
    );
}

#[test]
fn lowers_homogeneous_rest_tuples_to_arrays() {
    lower(
        r#"
        function total(values: [number, ...number[]]): number {
            let sum: number = 0;
            for (const value of values) sum += value;
            return sum;
        }
        function main(): void {
            console.log(total([1, 2, 3]));
        }
        "#,
    );
}

#[test]
fn lowers_heterogeneous_rest_tuples_to_json_arrays() {
    let program = lower(
        r#"
        function total(value: [string, ...number[]]): string {
            let sum: number = 0;
            for (let index: number = 1; index < value.length; index++) sum += value[index];
            return value[0] + ":" + String(sum);
        }
        function main(): void { console.log(total(["n", 1, 2, 3])); }
        "#,
    );
    assert_eq!(
        program.functions[0].params[0].ty,
        HirType::Array(Box::new(HirType::Json))
    );
}

#[test]
fn accepts_template_strings_array_for_tagged_templates() {
    lower(
        r#"function tag(parts: TemplateStringsArray, value: number): string {
            return parts[0] + String(value) + parts[1];
        }
        function main(): void { console.log(tag`value=${21}!`); }"#,
    );
}

#[test]
fn lowers_generator_to_a_lazy_array_producer() {
    let module = thaw_parser::parse_typescript(
        "function* values(): Generator<number> { yield 1; } function main(): void {}",
    )
    .unwrap();
    let program = lower_module(&module).unwrap();
    let values = program
        .functions
        .iter()
        .find(|function| function.name == "values")
        .unwrap();
    assert_eq!(
        values.ret,
        HirType::Function(
            vec![
                HirType::I64,
                HirType::Str,
                HirType::Undefined,
                HirType::Array(Box::new(HirType::F64)),
                HirType::Array(Box::new(HirType::F64)),
                HirType::Array(Box::new(HirType::F64)),
            ],
            Box::new(HirType::Array(Box::new(HirType::F64)))
        )
    );
}

#[test]
fn preserves_union_narrowing_after_assertion_calls() {
    lower(
        r#"
        type Value = string | number;
        function assertString(value: Value): asserts value is string {
            if (typeof value !== "string") throw new Error("not string");
        }
        function upper(value: Value): string {
            assertString(value);
            return value.toUpperCase();
        }
        function main(): void {
            console.log(upper("ok"));
        }
        "#,
    );
}

#[test]
fn narrows_unknown_values_with_typeof() {
    lower(
        r#"
        function show(value: unknown): string {
            if (typeof value === "string") return value.toUpperCase();
            if (typeof value === "number") return String(value * 2);
            return "other";
        }
        function main(): void {
            console.log(show("ok"), show(21), show(false));
        }
        "#,
    );
}

#[test]
fn validates_contextual_generic_arrow_constraints() {
    let module = thaw_parser::parse_typescript(
        r#"
        function main(): void {
            ["wrong"].map(<T extends number>(value: T): T => value);
        }
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("does not satisfy constraint F64"), "{error}");
}

#[test]
fn validates_generic_instantiation_expressions() {
    for (source, expected) in [
        (
            "function numeric<T extends number>(value: T): T { return value; } function main(): void { const bad = numeric<string>; }",
            "does not satisfy constraint F64",
        ),
        (
            "function plain(value: number): number { return value; } function main(): void { const bad = plain<number>; }",
            "non-generic function `plain`",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn validates_annotated_generic_arrow_constraints() {
    let module = thaw_parser::parse_typescript(
        r#"
        function main(): void {
            const invalid: (value: string) => string =
                <T extends number>(value: T): T => value;
        }
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("does not satisfy constraint F64"), "{error}");
}

#[test]
fn validates_local_generic_arrow_calls() {
    for (source, expected) in [
        (
            "function main(): void { const numeric = <T extends number>(value: T): T => value; numeric(\"wrong\"); }",
            "does not satisfy constraint F64",
        ),
        (
            "function main(): void { const pair = <T, U>(left: T, right: U): T => left; pair(1); }",
            "expects 2 argument(s), got 1",
        ),
        (
            "function main(): void { const identity = <T>(value: T): T => value; identity<string>(1); }",
            "explicit type is Str",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn validates_user_function_generic_callback_constraints() {
    let module = thaw_parser::parse_typescript(
        r#"
        function apply(callback: (value: string) => string, value: string): string {
            return callback(value);
        }
        function numeric<T extends number>(value: T): T { return value; }
        function main(): void { apply(numeric, "wrong"); }
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("cannot specialize generic callback `numeric`"),
        "{error}"
    );
    assert!(error.contains("does not satisfy constraint F64"), "{error}");
}

#[test]
fn validates_generic_function_type_alias_assignments() {
    for (source, expected) in [
        (
            "type Identity = <T>(value: T) => T; function main(): void { const bad: Identity = <U>(value: U): string => String(value); }",
            "incompatible with function type alias `Identity`",
        ),
        (
            "type Numeric = <T extends number>(value: T) => T; function main(): void { const bad: Numeric = <U extends string>(value: U): U => value; }",
            "constraints do not match function type alias `Numeric`",
        ),
        (
            "type Identity = <T>(value: T) => T; function bad<T>(value: T): string { return \"wrong\"; } function main(): void { const invalid: Identity = bad; }",
            "incompatible with function type alias `Identity`",
        ),
        (
            "type Identity = <T>(value: T) => T; type Stringify = <T>(value: T) => string; function main(): void { const identity: Identity = <T>(value: T): T => value; const invalid: Stringify = identity; }",
            "incompatible with function type alias `Stringify`",
        ),
        (
            "type Forward = Stringify; type Stringify = <T>(value: T) => string; function main(): void { const invalid: Forward = <T>(value: T): T => value; }",
            "incompatible with function type alias `Forward`",
        ),
        (
            "type Stringify = { <T>(value: T): string }; function main(): void { const invalid: Stringify = <T>(value: T): T => value; }",
            "incompatible with function type alias `Stringify`",
        ),
        (
            "type Identity = <T>(value: T) => T; function main(): void { const invalid: Identity = function<T>(value: T): string { return String(value); }; }",
            "incompatible with function type alias `Identity`",
        ),
        (
            "type Factory = <T = string>() => T; function main(): void { const invalid: Factory = <T = number>(): T => 1; }",
            "defaults do not match function type alias `Factory`",
        ),
        (
            "type Identity = <T>(value: T) => T; function main(): void { const invalid: Identity = <T>(value: T) => String(value); }",
            "incompatible with function type alias `Identity`",
        ),
        (
            "type Stringify = <T>(value: T) => string; function main(): void { const invalid: Stringify = <T>(value: T) => 1; }",
            "incompatible with function type alias `Stringify`",
        ),
        (
            "type Nullify = <T>(value: T) => null; function main(): void { const invalid: Nullify = <T>(value: T) => undefined; }",
            "incompatible with function type alias `Nullify`",
        ),
        (
            "type Identity = <T>(value: T) => T; function main(): void { const invalid: Identity = <T>(value: T) => \"value=\" + String(value); }",
            "incompatible with function type alias `Identity`",
        ),
        (
            "type Predicate = <T>(value: T, flag: boolean) => boolean; function main(): void { const invalid: Predicate = <T>(value: T, flag: boolean) => flag && \"wrong\"; }",
            "needs an explicit return type",
        ),
        (
            "type OptionalIdentity = <T>(value?: T) => T; function main(): void { const invalid: OptionalIdentity = <T>(value: T): T => value; }",
            "optional parameters do not match function type alias `OptionalIdentity`",
        ),
        (
            "type Choose = <T, U>(left: T, right: U) => T; function main(): void { const invalid: Choose = function<T, U>(left: T, right: U) { if (true) return left; return right; }; }",
            "needs an explicit return type",
        ),
        (
            "type Identity = <T>(value: T) => T; async function asynchronous<T>(value: T): Promise<T> { return value; } function main(): void { const invalid: Identity = asynchronous; }",
            "incompatible with function type alias `Identity`",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn validates_inline_generic_function_type_assignments() {
    let module = thaw_parser::parse_typescript(
        "function main(): void { const invalid: <T>(value: T) => T = <U>(value: U): string => String(value); }",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("inline generic function type"), "{error}");
    assert!(error.contains("incompatible"), "{error}");
}

#[test]
fn validates_generic_callable_interface_assignments() {
    for (source, name) in [
        (
            "interface Identity { <T>(value: T): T; } function main(): void { const invalid: Identity = <U>(value: U): string => \"wrong\"; }",
            "Identity",
        ),
        (
            "interface Derived extends Middle {} interface Middle extends Identity {} interface Identity { <T>(value: T): T; } function main(): void { const invalid: Derived = <U>(value: U): string => \"wrong\"; }",
            "Derived",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains(&format!(
                "incompatible with function type alias `{name}`"
            )),
            "{error}"
        );
    }
}

#[test]
fn enforces_repeated_generic_constraints_inside_arrays_and_interfaces() {
    let array = thaw_parser::parse_typescript(
        r#"
        function sameArrays<T>(left: T[], right: T[]): T[] { return left; }
        function main(): void { sameArrays([1], [2]); }
        "#,
    )
    .unwrap();
    assert!(lower_module(&array).is_ok());

    let pair = thaw_parser::parse_typescript(
        r#"
        interface Pair<T, U> { first: T; second: U; }
        function diagonal<T>(value: Pair<T, T>): T { return value.first; }
        function main(): void { diagonal({ first: 1, second: "wrong" }); }
        "#,
    )
    .unwrap();
    let error = lower_module(&pair).unwrap_err();
    assert!(error.contains("conflicting call-site types"), "{error}");
}

#[test]
fn diagnoses_uninferable_and_unsupported_generic_layouts() {
    let uninferable = thaw_parser::parse_typescript(
        r#"
        function phantom<T, U>(value: T): T { return value; }
        function main(): void { phantom(1); }
        "#,
    )
    .unwrap();
    let error = lower_module(&uninferable).unwrap_err();
    assert!(
        error.contains("cannot infer generic type parameter `U`"),
        "{error}"
    );

    let unsupported = thaw_parser::parse_typescript(
        r#"
        function identity<T>(value: T): T { return value; }
        function main(): void { identity(JSON.parse("null")); }
        "#,
    )
    .unwrap();
    let error = lower_module(&unsupported).unwrap_err();
    assert!(
        error.contains("cannot specialize for native layout Json"),
        "{error}"
    );
}

#[test]
fn extern_generics_accept_json_collection_layouts() {
    let module = thaw_parser::parse_typescript(
        r#"
        declare function map<T>(values: T): T;
        function main(): void { map(JSON.parse("[]")); }
        "#,
    )
    .unwrap();
    assert!(lower_module(&module).is_ok());
}

#[test]
fn extern_dynamic_parameters_contextually_type_callback_arguments() {
    let module = thaw_parser::parse_typescript(
        r#"
        declare function subscribe(callback: any): void;
        function main(): void {
            subscribe((error, value) => {
                if (error) console.log(error);
                console.log(value);
            });
        }
        "#,
    )
    .unwrap();
    assert!(lower_module(&module).is_ok());
}

#[test]
fn propagates_specializations_through_generic_function_calls() {
    let program = lower(
        r#"
        function forward<T, U>(first: T, second: U): T {
            return chooseFirst(first, second);
        }
        function chooseFirst<T, U>(first: T, second: U): T {
            return first;
        }
        function main(): void { console.log(forward(42, "unused")); }
        "#,
    );
    let forward = program
        .functions
        .iter()
        .find(|function| function.name == "forward__thaw_f64__str")
        .expect("outer specialization");
    assert!(format!("{:?}", forward.body).contains("chooseFirst__thaw_f64__str"));
    assert_eq!(
        program
            .functions
            .iter()
            .filter(|function| function.name == "chooseFirst__thaw_f64__str")
            .count(),
        1
    );
}

#[test]
fn specializes_type_variables_nested_in_arrays_and_objects() {
    let program = lower(
        r#"
        interface Box<T> { value: T; }
        interface Wrapper<T> { boxed: Box<T>; }
        function sameArray<T>(value: T[]): T[] { return value; }
        function sameBox<T>(value: { value: T }): { value: T } { return value; }
        function namedBox<T>(value: Box<T>): Box<T> { return value; }
        function wrapped<T>(value: Wrapper<T>): Wrapper<T> { return value; }
        function main(): void {
            sameArray([1, 2]);
            sameBox({ value: 3 });
            namedBox({ value: 4 });
            wrapped({ boxed: { value: 5 } });
        }
        "#,
    );
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == "sameArray__thaw_array_f64"));
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == "sameBox__thaw_object_value_f64"));
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == "namedBox__thaw_object_value_f64"));
    assert!(program
        .functions
        .iter()
        .any(|function| { function.name == "wrapped__thaw_object_boxed_object_value_f64" }));
}

#[test]
fn specializes_named_structures_with_multiple_type_parameters() {
    let program = lower(
        r#"
        interface Pair<T, U> { first: T; second: U; }
        function samePair<T, U>(value: Pair<T, U>): Pair<T, U> { return value; }
        function main(): void { samePair({ first: 1, second: "two" }); }
        "#,
    );
    let pair = program
        .functions
        .iter()
        .find(|function| function.name == "samePair__thaw_object_first_f64_second_str")
        .expect("Pair<number, string> specialization");
    assert_eq!(
        pair.params[0].ty,
        HirType::Object(vec![
            ("first".into(), HirType::F64),
            ("second".into(), HirType::Str),
        ])
    );
    assert_eq!(pair.ret, pair.params[0].ty);
}

#[test]
fn specializes_generic_property_projections() {
    let program = lower(
        r#"
        interface Pair<T, U> { first: T; second: U; }
        interface Box<T> { value: T; }
        interface Wrapper<T> { boxed: Box<T>; }
        function first<T, U>(value: Pair<T, U>): T { return value.first; }
        function unbox<T>(value: Wrapper<T>): T { return value.boxed.value; }
        function main(): void {
            const n = first({ first: 1, second: "two" });
            const deep = unbox({ boxed: { value: 3 } });
        }
        "#,
    );
    let first = program
        .functions
        .iter()
        .find(|function| function.name == "first__thaw_object_first_f64_second_str")
        .unwrap();
    assert_eq!(first.ret, HirType::F64);
    let unbox = program
        .functions
        .iter()
        .find(|function| function.name == "unbox__thaw_object_boxed_object_value_f64")
        .unwrap();
    assert_eq!(unbox.ret, HirType::F64);
}

#[test]
fn resolves_forward_type_aliases_and_rejects_cycles() {
    let program = lower(
        r#"type Later = Base & { count: number };
           type Base = { name: string };
           type Choice = string | number;
           function choose(value: Choice): Later {
               return { count: 2, name: "alias" };
           }"#,
    );
    assert_eq!(
        program.functions[0].params[0].ty,
        HirType::Union(vec![HirType::Str, HirType::F64])
    );
    assert_eq!(
        program.functions[0].ret,
        HirType::Object(vec![
            ("name".into(), HirType::Str),
            ("count".into(), HirType::F64),
        ])
    );

    let module = thaw_parser::parse_typescript("type A = B; type B = A;").unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("type declaration cycle"));

    let mixed = lower(
        r#"interface Item { label: Label; count: Count }
           type Count = number;
           type Label = string;
           function item(value: Item): string { return value.label; }"#,
    );
    assert_eq!(
        mixed.functions[0].params[0].ty,
        HirType::Object(vec![
            ("label".into(), HirType::Str),
            ("count".into(), HirType::F64),
        ])
    );

    let module =
        thaw_parser::parse_typescript("type Link = Node; interface Node { next: Link; }")
            .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("self-referential"));
}

#[test]
fn validates_generic_type_alias_instantiations() {
    let module = thaw_parser::parse_typescript(
        "type Boxed<T> = { value: T }; function bad(value: Boxed<number, string>): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("expects 1 type argument(s), got 2"));

    let module = thaw_parser::parse_typescript(
        "type Loop<T> = Loop<T>; function bad(value: Loop<number>): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("generic type alias `Loop` is (indirectly) self-referential"));

    let module = thaw_parser::parse_typescript(
        "type Numeric<T extends number> = { value: T }; function bad(value: Numeric<string>): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("does not satisfy constraint F64"));

    let module = thaw_parser::parse_typescript(
        "type Missing = NonNullable<null | undefined>; function bad(value: Missing): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("NonNullable<T> has no native value"));

    let module = thaw_parser::parse_typescript(
        "type Dynamic = Record<string, number>; function accepts(value: Dynamic): void {}",
    )
    .unwrap();
    let program = lower_module(&module).unwrap();
    assert_eq!(
        program.functions[0].params[0].ty,
        HirType::Dictionary(Box::new(HirType::F64))
    );

    let module = thaw_parser::parse_typescript(
        "type Bad = Pick<{ value: number }, \"missing\">; function bad(value: Bad): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("key `missing` does not exist"));

    let module = thaw_parser::parse_typescript(
        "type Bad = { value: number }[\"missing\"]; function bad(value: Bad): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("indexed access key `missing` does not exist"));
}

#[test]
fn lowers_constructor_signatures_and_generic_aliases() {
    let program = lower(
        r#"type Factory<T> = new (value: T) => { value: T };
           function accept(factory: Factory<number>): void {}"#,
    );
    assert_eq!(
        program.functions[0].params[0].ty,
        HirType::Function(
            vec![HirType::F64],
            Box::new(HirType::Object(vec![("value".into(), HirType::F64)])),
        )
    );
}

#[test]
fn lowers_string_index_signatures_as_typed_dictionaries() {
    let program = lower(
        r#"
        type Scores = { [key: string]: number; total: number };
        interface Labels<T> { [key: string]: T; primary: T }

        function sum(scores: Scores, key: string): number {
            scores[key] += 2;
            scores.total++;
            return scores[key] + scores.total;
        }

        function label(values: Labels<string>, key: string): string {
            values[key] = "ready";
            return values.primary + values[key];
        }
        "#,
    );
    assert_eq!(
        program.functions[0].params,
        vec![
            HirParam {
                name: "scores".into(),
                ty: HirType::Dictionary(Box::new(HirType::F64)),
            },
            HirParam {
                name: "key".into(),
                ty: HirType::Str,
            },
        ]
    );
    assert_eq!(
        program.functions[1].params[0].ty,
        HirType::Dictionary(Box::new(HirType::Str))
    );
}

#[test]
fn validates_generic_interface_defaults_and_constraints() {
    let module = thaw_parser::parse_typescript(
        "interface Numeric<T extends number> { value: T } function bad(value: Numeric<string>): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("does not satisfy constraint F64"));
}

#[test]
fn validates_generic_interface_inherited_field_collisions() {
    let module = thaw_parser::parse_typescript(
        "interface Base<T> { value: T } interface Child<T> extends Base<T> { value: string } function bad(value: Child<number>): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("incompatible type"));
}

#[test]
fn validates_concrete_generic_base_constraints() {
    let module = thaw_parser::parse_typescript(
        "interface Bad extends Numeric<string> {} interface Numeric<T extends number> { value: T } function bad(value: Bad): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("does not satisfy constraint F64"));
}

#[test]
fn rejects_non_object_generic_alias_base() {
    let module = thaw_parser::parse_typescript(
        "type Value<T> = T; interface Bad extends Value<number> {} function bad(value: Bad): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("can only extend object-shaped"));
}

#[test]
fn lowers_interface_as_a_named_object_type() {
    let program = lower(
        r#"interface Point {
            x: number;
            y: number;
        }

        function dist(p: Point): number {
            return p.x + p.y;
        }

        function main(): void {
            const p: Point = { y: 2, x: 1 };
            console.log(dist(p));
        }"#,
    );

    let point_ty =
        HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]);

    let dist = &program.functions[0];
    assert_eq!(
        dist.params,
        vec![HirParam {
            name: "p".into(),
            ty: point_ty.clone()
        }]
    );

    let main = &program.functions[1];
    // Declared via the interface name; the literal keeps its source field order as the
    // physical layout and the interface's `{ x, y }` order is realized by the registered
    // layout view (same machinery as inline object type literals).
    assert_eq!(
        unstage_let(&main.body[0]),
        HirStmt::Let(
            "p".into(),
            point_ty,
            HirExpr::ObjectLit(vec![
                ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
                ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
            ]),
        )
    );
}

#[test]
fn interfaces_can_reference_each_other_regardless_of_declaration_order() {
    // `Line` is declared before `Point`, and refers to it -- the
    // resolver must not depend on source order.
    let program = lower(
        r#"interface Line {
            start: Point;
            length: number;
        }

        interface Point {
            x: number;
            y: number;
        }

        function main(): void {
            const l: Line = { start: { x: 1, y: 2 }, length: 5 };
            console.log(l.length);
        }"#,
    );

    let point_ty =
        HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]);
    let line_ty = HirType::Object(vec![
        ("start".into(), point_ty),
        ("length".into(), HirType::F64),
    ]);

    assert_eq!(
        unstage_let(&program.functions[0].body[0]),
        HirStmt::Let(
            "l".into(),
            line_ty,
            HirExpr::ObjectLit(vec![
                (
                    "start".into(),
                    HirExpr::ObjectLit(vec![
                        ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                        ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
                    ]),
                ),
                ("length".into(), HirExpr::Lit(HirLit::F64(5.0))),
            ]),
        )
    );
}

#[test]
fn rejects_self_referential_interface() {
    let module = thaw_parser::parse_typescript(
        r#"interface Node {
            value: number;
            next: Node;
        }
        function main(): void {}"#,
    )
    .unwrap();
    let err = lower_module(&module).unwrap_err();
    assert!(err.contains("self-referential"), "unexpected error: {err}");
}

#[test]
fn lowers_generic_interface_instantiated_with_a_concrete_type() {
    let program = lower(
        r#"interface Box<T> {
            value: T;
        }
        function unwrap(b: Box<number>): number {
            return b.value;
        }
        function main(): void {
            const b: Box<number> = { value: 5 };
            console.log(unwrap(b));
        }"#,
    );

    let box_number_ty = HirType::Object(vec![("value".into(), HirType::F64)]);
    assert_eq!(
        program.functions[0].params,
        vec![HirParam {
            name: "b".into(),
            ty: box_number_ty.clone()
        }]
    );
    assert_eq!(
        unstage_let(&program.functions[1].body[0]),
        HirStmt::Let(
            "b".into(),
            box_number_ty,
            HirExpr::ObjectLit(vec![("value".into(), HirExpr::Lit(HirLit::F64(5.0)))]),
        )
    );
}

#[test]
fn generic_interface_instantiations_with_different_arguments_are_distinct_shapes() {
    let program = lower(
        r#"interface Box<T> { value: T; }
        function f(a: Box<number>, b: Box<string>): void {}
        function main(): void {}"#,
    );
    assert_eq!(
        program.functions[0].params[0].ty,
        HirType::Object(vec![("value".into(), HirType::F64)])
    );
    assert_eq!(
        program.functions[0].params[1].ty,
        HirType::Object(vec![("value".into(), HirType::Str)])
    );
}

#[test]
fn generic_interface_field_can_be_an_array_or_object_literal_of_the_type_parameter() {
    let program = lower(
        r#"interface Box<T> {
            items: T[];
        }
        function main(): void {
            const b: Box<number> = { items: [1, 2, 3] };
            console.log(b.items.length);
        }"#,
    );
    let box_ty = HirType::Object(vec![(
        "items".into(),
        HirType::Array(Box::new(HirType::F64)),
    )]);
    assert!(matches!(&program.functions[0].body[0], HirStmt::Let(_, ty, _) if *ty == box_ty));
}

#[test]
fn rejects_wrong_number_of_generic_type_arguments() {
    let module = thaw_parser::parse_typescript(
        r#"interface Pair<A, B> { first: A; second: B; }
        function main(): void {
            const p: Pair<number> = { first: 1, second: 2 };
        }"#,
    )
    .unwrap();
    let err = lower_module(&module).unwrap_err();
    assert!(err.contains("type argument"), "unexpected error: {err}");
}

#[test]
fn rejects_self_referential_generic_interface() {
    // Triggered via a parameter type (not a `let`) so the error comes
    // from resolving `Node<number>` itself, not from lowering some
    // initializer expression first.
    let module = thaw_parser::parse_typescript(
        r#"interface Node<T> {
            value: T;
            next: Node<T>;
        }
        function f(n: Node<number>): void {}
        function main(): void {}"#,
    )
    .unwrap();
    let err = lower_module(&module).unwrap_err();
    assert!(err.contains("self-referential"), "unexpected error: {err}");
}

#[test]
fn interface_extends_prepends_base_fields() {
    let program = lower(
        r#"interface Shape {
            color: number;
        }
        interface Circle extends Shape {
            radius: number;
        }
        function main(): void {
            const c: Circle = { color: 1, radius: 2 };
            console.log(c.radius);
        }"#,
    );

    let circle_ty = HirType::Object(vec![
        ("color".into(), HirType::F64),
        ("radius".into(), HirType::F64),
    ]);
    assert_eq!(
        unstage_let(&program.functions[0].body[0]),
        HirStmt::Let(
            "c".into(),
            circle_ty,
            HirExpr::ObjectLit(vec![
                ("color".into(), HirExpr::Lit(HirLit::F64(1.0))),
                ("radius".into(), HirExpr::Lit(HirLit::F64(2.0))),
            ]),
        )
    );
}

#[test]
fn interface_can_extend_multiple_bases_in_order() {
    let program = lower(
        r#"interface A { a: number; }
        interface B { b: number; }
        interface C extends A, B {
            c: number;
        }
        function main(): void {
            const v: C = { a: 1, b: 2, c: 3 };
            console.log(v.a);
        }"#,
    );
    let c_ty = HirType::Object(vec![
        ("a".into(), HirType::F64),
        ("b".into(), HirType::F64),
        ("c".into(), HirType::F64),
    ]);
    assert_eq!(
        unstage_let(&program.functions[0].body[0]),
        HirStmt::Let(
            "v".into(),
            c_ty,
            HirExpr::ObjectLit(vec![
                ("a".into(), HirExpr::Lit(HirLit::F64(1.0))),
                ("b".into(), HirExpr::Lit(HirLit::F64(2.0))),
                ("c".into(), HirExpr::Lit(HirLit::F64(3.0))),
            ]),
        )
    );
}

#[test]
fn records_arrow_capture_names_and_types() {
    let program = lower(
        r#"function main(): void {
            const base: number = 40;
            const add = (value: number): number => base + value;
            console.log(add(2));
        }"#,
    );
    let HirStmt::Let(_, _, HirExpr::Lambda(captures, _, _, _)) = &program.functions[0].body[1]
    else {
        panic!("expected captured lambda");
    };
    assert_eq!(
        captures,
        &[HirParam {
            name: "base".into(),
            ty: HirType::F64,
        }]
    );
}

#[test]
fn map_accepts_object_and_array_keys_but_rejects_function_keys() {
    let program = lower(
        r#"function main(): void {
            const byPoint: Map<{ x: number }, string> = new Map<{ x: number }, string>();
            const byRow: Map<number[], string> = new Map<number[], string>();
            console.log(byPoint.size + byRow.size);
        }"#,
    );
    assert!(matches!(
        program.functions[0].body[0],
        HirStmt::Let(_, HirType::Map(_, _), _)
    ));

    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const byCallback: Map<() => void, string> = new Map<() => void, string>();
        }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("Map/Set keys must be"), "{error}");
}

#[test]
fn weak_map_accepts_object_keys_but_rejects_primitive_keys() {
    let program = lower(
        r#"function main(): void {
            const cache: WeakMap<{ x: number }, string> = new WeakMap<{ x: number }, string>();
            const key = { x: 1 };
            cache.set(key, "value");
            console.log(cache.get(key));
        }"#,
    );
    assert!(matches!(
        program.functions[0].body[0],
        HirStmt::Let(_, HirType::WeakMap(_, _), _)
    ));

    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const byString: WeakMap<string, number> = new WeakMap<string, number>();
        }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("WeakMap/WeakSet keys must be"), "{error}");

    for operation in [
        "console.log(cache.size);",
        "cache.clear();",
        "cache.keys();",
        "cache.forEach((value: string) => console.log(value));",
        "for (const entry of cache) console.log(entry);",
    ] {
        let source = format!(
            "function main(): void {{ const cache: WeakMap<{{ x: number }}, string> = new WeakMap<{{ x: number }}, string>(); {operation} }}"
        );
        let module = thaw_parser::parse_typescript(&source).unwrap();
        assert!(lower_module(&module).is_err(), "unexpectedly accepted {operation}");
    }
}

#[test]
fn never_calls_terminate_the_enclosing_return_path() {
    let program = lower(
        r#"function fail(message: string): never { throw new Error(message); }
        function value(flag: boolean): string {
            if (flag) return "ok";
            return fail("bad");
        }
        function main(): void { console.log(value(true)); }"#,
    );
    let body = &program.functions[1].body;
    assert!(matches!(body[1], HirStmt::Expr(HirExpr::Call(_, _))));
    assert!(matches!(body[2], HirStmt::Throw(_)));
}

#[test]
fn explicit_this_parameters_are_forwarded_by_function_call() {
    let program = lower(
        r#"function label(this: { prefix: string }, value: number): string {
            return this.prefix + String(value);
        }
        function main(): void {
            const context = { prefix: "item-" };
            console.log(label.call(context, 42));
        }"#,
    );
    assert_eq!(program.functions[0].params[0].name, "__thaw_this");
    let HirStmt::Expr(HirExpr::Call(_, arguments)) = &program.functions[1].body[1] else {
        panic!("expected console call");
    };
    assert!(matches!(arguments[0], HirExpr::Call(_, ref values) if values.len() == 2));

    let bound = lower(
        r#"function label(this: { prefix: string }, value: number): string {
            return this.prefix + String(value);
        }
        function main(): void {
            const bound = label.bind({ prefix: "item-" });
            console.log(bound(42));
        }"#,
    );
    assert!(matches!(
        bound.functions[1].body[0],
        HirStmt::Let(_, HirType::Function(ref params, _), _) if params == &[HirType::F64]
    ));
}

#[test]
fn function_expression_receiver_provenance_survives_contextual_lowering() {
    let program = lower(
        r#"function main(): void {
            const ordinary: (value: number) => number = function(value) { return value; };
            const lexical: (value: number) => number = value => value;
            console.log(ordinary(1), lexical(2));
        }"#,
    );
    let body = &program.functions[0].body;
    assert!(matches!(
        &body[0],
        HirStmt::Let(_, _, HirExpr::NonArrowFunction(_))
    ));
    assert!(matches!(&body[1], HirStmt::Let(_, _, HirExpr::Lambda(..))));
}

#[test]
fn generic_function_expression_keeps_receiver_provenance_after_specialization() {
    let program = lower(
        r#"function main(): void {
            const identity = function<T>(value: T): T { return value; };
            console.log(identity(7));
        }"#,
    );
    assert!(format!("{:?}", program.functions[0].body).contains("NonArrowFunction"));
}

#[test]
fn merges_compatible_interface_declarations() {
    lower(
        r#"interface User { name: string }
        interface User { age: number }
        function main(): void {
            const user: User = { name: "A", age: 1 };
            console.log(user.name, user.age);
        }"#,
    );
}

#[test]
fn accepts_undefined_for_an_optional_callable_annotation() {
    lower(
        r#"type Handler = ((value: number) => number) | undefined;
        function main(): void {
            let handler: Handler = undefined;
            console.log(handler?.(20) ?? 42);
        }"#,
    );
}

#[test]
fn rewritten_external_type_name_resolves_to_jsvalue_bare_and_generic() {
    // The module graph renames an imported external class/interface used
    // in type position to a `__thaw_`-prefixed rewrite symbol. That name
    // has no thaw-modelled type; resolving it (with or without type
    // arguments -- `Context` vs `Context<Env, Params>`) must degrade to
    // `JsValue`, not fail the build. A `JsValue` parameter admits a
    // dynamic member call; a failed resolution would abort lowering.
    lower(
        r#"
        function bare(value: __thaw_typed_js_abc123): string {
            return value.text("hi");
        }
        function generic(value: __thaw_type_pkg_Ctx<number, string>): string {
            return value.text("bye");
        }
        function main(): void {
            console.log(bare(getDynamicValue("x")), generic(getDynamicValue("y")));
        }
        "#,
    );
}

#[test]
fn buffer_is_a_distinct_type_that_erases_to_a_number_array() {
    // `Buffer` / `Uint8Array` -> `HirType::Bytes` during lowering
    // (type-checks and would dispatch differently), but the erase pass
    // collapses it to `Array(F64)` before the program leaves lowering,
    // so codegen -- and this assertion -- never see `Bytes`.
    let program = lower(
        r#"
        declare function raw(): Buffer;
        function takesArray(a: number[]): number { return a.length; }
        function nested(): void {
            const pair: Uint8Array[] = [raw(), raw()];
            const opt: Uint8Array | undefined = raw();
        }
        function main(): void {
            const b: Uint8Array = raw();
            let sum = 0;
            for (const byte of b) { sum = sum + byte; }
            const asArray: number[] = b;
            const backToBuf: Buffer = asArray;
            console.log(b.length, b[0], sum, takesArray(b), backToBuf.length);
        }
        "#,
    );
    assert!(
        !format!("{program:?}").contains("Bytes"),
        "a `Bytes` marker survived the erase pass into the lowered program"
    );
}

#[test]
fn unannotated_buffer_binding_still_dispatches_byte_specific_methods() {
    // Storing a `let`/`const` binding's *declared* type from the erasing
    // `infer_expr_type` (instead of `infer_expr_type_inner`) used to
    // permanently downgrade an unannotated `Bytes` value to `Array(F64)`
    // in `scope` the moment it was bound -- `.length`/spread/indexing
    // still worked (they're written for `Array` either way), but a
    // byte-specific method (`bytes_methods.rs`) or `Buffer.isBuffer`
    // (`static_builtins.rs`) misdispatched or hard-errored, since both
    // read the receiver's type back out of `scope`. This would fail to
    // even lower (`` `.readUInt8()` is only supported on a Buffer /
    // Uint8Array ``) without the fix.
    let program = lower(
        r#"
        declare function raw(): Buffer;
        function main(): void {
            const b = raw();
            console.log(Buffer.isBuffer(b), b.readUInt8(0));
        }
        "#,
    );
    assert!(
        !format!("{program:?}").contains("Bytes"),
        "a `Bytes` marker survived the erase pass into the lowered program"
    );
}

#[test]
fn function_expression_this_is_hidden_from_public_signature() {
    let program = lower(
        r#"function main(): void {
            const add: (delta: number) => number = function(this: Json, delta: number): number {
                return Number(this["base"]) + delta;
            };
            const holder: Json = { base: 7 };
            console.log(add.call(holder, 3));
        }"#,
    );
    let HirStmt::Let(_, HirType::Function(visible, _), HirExpr::NonArrowFunction(closure)) =
        &program.functions[0].body[0] else { panic!("expected typed non-arrow function expression"); };
    assert_eq!(visible, &[HirType::F64]);
    let HirExpr::Lambda(_, params, _, _) = closure.as_ref() else { panic!("expected lambda body"); };
    assert_eq!(params[0].name, "__thaw_this");
    assert_eq!(params[0].ty, HirType::Json);
    assert_eq!(params.len(), 2);
}

#[test]
fn unused_function_expression_this_keeps_visible_arity() {
    let program = lower(
        r#"function main(): void {
            const add = function(delta: number): number { return delta + 1; };
            console.log(add(2));
        }"#,
    );
    let HirStmt::Let(_, _, HirExpr::NonArrowFunction(closure)) = &program.functions[0].body[0]
        else { panic!("expected non-arrow function expression"); };
    let HirExpr::Lambda(_, params, _, _) = closure.as_ref() else { panic!("expected lambda body"); };
    assert_eq!(params.len(), 1);
}

#[test]
fn generic_non_arrow_this_annotation_is_retained_until_specialization() {
    lower(r#"function main(): void {
        const choose = function<T>(this: T, value: T): T { return this; };
        const alias = choose;
        console.log(alias.call(7, 7));
        console.log(alias.apply(8, [8] as [number]));
        const bound = alias.bind(9);
        console.log(bound(9));
    }"#);
}

#[test]
fn generic_non_arrow_this_scope_and_empty_operations_lower() {
    lower(r#"function main(): void {
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
    }"#);
}

#[test]
fn generic_non_arrow_optional_this_operations_lower() {
    lower(r#"function main(): void {
        const choose = function<T>(this: T, value?: T): T { return this; };
        console.log(choose.call(7));
        console.log(choose.apply(8, null));
        console.log(choose.apply(9, undefined));
        const bound = choose.bind(10);
        console.log(bound());
        console.log(bound(10));
    }"#);
}

#[test]
fn generic_non_arrow_rest_this_operations_lower() {
    lower(r#"function main(): void {
        const count = function<T>(this: T, ...values: T[]): number {
            return values.length;
        };
        console.log(count.call(7, 7, 7));
        console.log(count.apply(8, [8, 8] as [number, number]));
        const bound = count.bind(9);
        console.log(bound(9, 9));
        const partial = count.bind(10, 10);
        console.log(partial(10));
    }"#);
}

#[test]
fn generic_non_arrow_apply_runtime_nullish_tuple_preserves_order() {
    lower(r#"function main(): void {
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
    }"#);
}

#[test]
fn non_arrow_receiver_nullability_and_disjoint_union_lower() {
    lower(r#"function main(): void {
        const optional = function(this: number | undefined): string { return typeof this; };
        const nullable = function(this: number | null): string { return typeof this; };
        const nullish = function(this: number | null | undefined): string { return typeof this; };
        const choice = function(this: number | string): string { return typeof this; };
        const jsonOptional = function(this: Json | undefined): string { return typeof this; };
        console.log(optional(), optional.call(5), optional.call(undefined));
        console.log(nullable.call(null), nullable.call(6));
        console.log(nullish(), nullish.call(null), nullish.call(7));
        console.log(choice.call(8), choice.call("x"));
        console.log(jsonOptional(), jsonOptional.call({ value: 1 } as Json));
    }"#);
}
#[test]
fn non_arrow_string_literal_union_receiver_lowers() {
    lower(r#"type Choice<T> = "open" | T;
    type Plain = "open" | "closed";
    function main(): void {
        const exact = function(this: "open" | "closed"): string { return typeof this; };
        console.log(exact.call("open"), exact.call("closed"));
        const wide = function(this: string | "open"): string { return typeof this; };
        console.log(wide.call("other"));
        const json = function(this: Json | "open"): string { return typeof this; };
        console.log(json.call("other"));
        const genericAlias = function(this: Choice<"closed">): string { return typeof this; };
        console.log(genericAlias.call("open"), genericAlias.call("closed"));
        const nestedAlias = function(this: Choice<Choice<"closed">>): string { return typeof this; };
        console.log(nestedAlias.call("open"), nestedAlias.call("closed"));
        const plainAlias = function(this: Plain): string { return typeof this; };
        console.log(plainAlias.call("open"), plainAlias.call("closed"));
    }"#);

    let circular = thaw_parser::parse_typescript(r#"
        type Cycle = "open" | Cycle;
        function main(): void {
            const bad = function(this: Cycle): string { return typeof this; };
            console.log(bad.call("open"));
        }
    "#).unwrap();
    let error = lower_module(&circular).unwrap_err();
    assert!(error.contains("cycle"), "unexpected error: {error}");

    let generic_cycle = thaw_parser::parse_typescript(r#"
        type Loop<T> = "open" | Loop<T>;
        function main(): void {
            const bad = function(this: Loop<"closed">): string { return typeof this; };
            console.log(bad.call("open"));
        }
    "#).unwrap();
    let error = lower_module(&generic_cycle).unwrap_err();
    assert!(error.contains("receiver type alias `Loop` is (indirectly) self-referential"),
        "unexpected error: {error}");

    let default_cycle = thaw_parser::parse_typescript(r#"
        type DefaultLoop<T = DefaultLoop> = "open" | T;
        function main(): void {
            const bad = function(this: DefaultLoop): string { return typeof this; };
            console.log(bad.call("open"));
        }
    "#).unwrap();
    let error = lower_module(&default_cycle).unwrap_err();
    assert!(error.contains("receiver type alias `DefaultLoop` is (indirectly) self-referential"),
        "unexpected error: {error}");
}

#[test]
fn non_arrow_nested_receiver_wrappers_lower_to_leaf_abi() {
    lower(r#"type Maybe<T> = T | undefined;
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
        }"#);
}

#[test]
fn byte_method_receiver_binding_precedes_normal_and_spread_arguments() {
    for operation in [
        "receiver().readUInt16LE(offset())",
        "receiver().writeUInt16LE(value(), offset())",
        "receiver().readUIntLE(offset(), width())",
        "receiver().writeUIntLE(value(), offset(), width())",
        "receiver().readBigUInt64LE(offset())",
        "receiver().writeBigUInt64LE(big(), offset())",
        "receiver().copy(target(), offset())",
        "receiver().set(target(), offset())",
        "receiver().indexOf(text(), offset())",
        "receiver().setFromHex(text())",
        "receiver().setFromBase64(text())",
        "receiver().writeUInt16LE(...[value(), offset()])",
        "receiver().set(...[target(), offset()])",
    ] {
        let program = lower(&format!(
            r#"
            declare function receiver(): Buffer;
            declare function target(): Buffer;
            declare function value(): number;
            declare function offset(): number;
            declare function width(): number;
            declare function big(): bigint;
            declare function text(): string;
            function main(): void {{ {operation}; }}
            "#,
        ));
        let main = program.functions.iter().find(|function| function.name == "main").unwrap();
        let body = format!("{:?}", main.body);
        let receiver_at = body.find("__thaw_bytes_receiver_").expect("missing byte receiver binding");
        let argument_at = body.find("__thaw_native_arg_").expect("missing argument binding");
        assert!(receiver_at < argument_at, "{operation}: receiver must evaluate before arguments: {body}");
    }
}

#[test]
fn throwing_byte_receiver_is_bound_before_side_effecting_spread_arguments() {
    let program = lower(
        r#"
        declare function sideEffect(): number;
        function throwingReceiver(): Buffer { throw new Error("receiver"); }
        function main(): void {
            throwingReceiver().writeUInt16LE(...[sideEffect(), sideEffect()]);
        }
        "#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let body = format!("{:?}", main.body);
    let receiver_at = body.find("__thaw_bytes_receiver_").unwrap();
    let argument_at = body.find("__thaw_native_arg_").unwrap();
    assert!(receiver_at < argument_at, "a throwing receiver must prevent argument evaluation: {body}");
}

#[test]
fn optional_call_preserves_nullable_and_nullish_result_tags() {
    lower(r#"
        type Maybe = (() => number | null) | undefined;
        type Both = (() => number | null | undefined) | undefined;
        function main(): void {
            let f: Maybe = undefined;
            let g: Both = undefined;
            const a: number | null | undefined = f?.();
            const b: number | null | undefined = g?.();
            let o: { method: () => number | null } | undefined = undefined;
            const c: number | null | undefined = o?.method();
            console.log(a, b, c);
        }
    "#);
}

#[test]
fn resolves_keyof_alias_target_before_its_owner_alias() {
    let module = thaw_parser::parse_typescript(
        "type I = { a: number }; type Keys = keyof I; function main(): void { const key: Keys = 'a'; console.log(key); }",
    )
    .unwrap();
    let mut aliases = HashMap::new();
    for item in &module.body {
        if let ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(alias))) = item {
            aliases.insert(alias.id.sym.to_string(), alias.as_ref());
        }
    }
    let mut resolved = HashMap::new();
    assert_eq!(
        resolve_named_type(
            "Keys",
            &HashMap::new(),
            &aliases,
            &GenericInterfaces::new(),
            &mut resolved,
            &mut Vec::new(),
        )
        .unwrap(),
        HirType::Str,
    );
    assert_eq!(
        resolved.get("I"),
        Some(&HirType::Object(vec![("a".to_string(), HirType::F64)])),
    );
    lower("type I = { a: number }; type Keys = keyof I; function main(): void { const key: Keys = 'a'; console.log(key); }");
}

#[test]
fn typeof_nullish_narrowing_keeps_null_until_it_is_excluded() {
    let unsafe_undefined = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined): number {
            if (typeof x !== "undefined") return x.value;
            return 0;
        }
    "#).unwrap();
    assert!(lower_module(&unsafe_undefined).is_err());

    let unsafe_object = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined): number {
            if (typeof x === "object") return x.value;
            return 0;
        }
    "#).unwrap();
    assert!(lower_module(&unsafe_object).is_err());

    let safe = lower(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined): number {
            if (typeof x === "undefined") return 0;
            if (x === null) return 1;
            return x.value;
        }
        function choose(x: Item | null | undefined): number {
            return typeof x === "undefined" ? 0 : x === null ? 1 : x.value;
        }
        function objectBranch(x: Item | null | undefined): number {
            if (typeof x === "object") {
                if (x !== null) return x.value;
            }
            return 0;
        }
    "#);
    let body = format!("{:?}", safe.functions);
    assert!(body.contains("NullableNone"), "{body}");
    assert!(body.contains("NullishIsNull"), "{body}");

    let reassigned = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined): number {
            if (typeof x !== "undefined") {
                x = null;
                return x.value;
            }
            return 0;
        }
    "#).unwrap();
    assert!(lower_module(&reassigned).is_err());
}

#[test]
fn typeof_partial_narrowing_tracks_nested_writes_without_losing_sibling_guards() {
    let stale_after_block = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined, flag: boolean): number {
            if (typeof x === "undefined") return 0;
            if (flag) { x = undefined; }
            if (x === null) return 1;
            return x.value;
        }
    "#).unwrap();
    assert!(lower_module(&stale_after_block).is_err());

    let stale_after_expression = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined, flag: boolean): number {
            if (typeof x === "undefined") return 0;
            const ignored = flag ? (x = undefined, 0) : 1;
            if (x === null) return 1;
            return x.value;
        }
    "#).unwrap();
    assert!(lower_module(&stale_after_expression).is_err());

    lower(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined, flag: boolean): number {
            if (typeof x === "undefined") return 0;
            if (flag) { x = undefined; }
            else { if (x === null) return 1; return x.value; }
            return 0;
        }
        function choose(x: Item | null | undefined, flag: boolean): number {
            if (typeof x === "undefined") return 0;
            return flag ? (x = undefined, 0) : x === null ? 1 : x.value;
        }
        function shadowed(x: Item | null | undefined, flag: boolean): number {
            if (typeof x === "undefined") return 0;
            if (flag) { let x: Item | null | undefined = undefined; x = undefined; }
            if (x === null) return 1;
            return x.value;
        }
    "#);
}

#[test]
fn completed_typeof_nullish_narrowing_does_not_survive_nested_writes() {
    let stale_after_completion = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined, flag: boolean): number {
            if (typeof x === "undefined") return 0;
            if (x === null) return 1;
            if (flag) { x = undefined; }
            return x.value;
        }
    "#).unwrap();
    assert!(lower_module(&stale_after_completion).is_err());

    let stale_after_expression = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined, flag: boolean): number {
            if (typeof x === "undefined") return 0;
            if (x === null) return 1;
            const ignored = flag ? (x = undefined, 0) : 1;
            return x.value;
        }
    "#).unwrap();
    assert!(lower_module(&stale_after_expression).is_err());

    lower(r#"
        type Item = { value: number };
        function branch(x: Item | null | undefined, flag: boolean): number {
            if (typeof x === "undefined") return 0;
            if (x === null) return 1;
            if (flag) { x = undefined; }
            else { return x.value; }
            return 0;
        }
        function choice(x: Item | null | undefined, flag: boolean): number {
            if (typeof x === "undefined") return 0;
            if (x === null) return 1;
            return flag ? (x = undefined, 0) : x.value;
        }
        function shadowed(x: Item | null | undefined, flag: boolean): number {
            if (typeof x === "undefined") return 0;
            if (x === null) return 1;
            if (flag) { let x: Item | null | undefined = undefined; x = undefined; }
            return x.value;
        }
    "#);
}

#[test]
fn typeof_guard_does_not_reapply_after_its_condition_writes_the_binding() {
    let stale_if = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined): number {
            if (typeof x !== "undefined" && (x = undefined, true)) {
                if (x !== null) return x.value;
            }
            return 0;
        }
    "#).unwrap();
    assert!(lower_module(&stale_if).is_err());

    let stale_guard_clause = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined): number {
            if (!(typeof x !== "undefined" && (x = undefined, true))) return 0;
            if (x !== null) return x.value;
            return 0;
        }
    "#).unwrap();
    assert!(lower_module(&stale_guard_clause).is_err());

    let stale_conditional = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined): number {
            return (typeof x !== "undefined" && (x = undefined, true))
                ? x.value : 0;
        }
    "#).unwrap();
    assert!(lower_module(&stale_conditional).is_err());

    lower(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined, flag: boolean): number {
            if (typeof x !== "undefined" && flag) {
                if (x !== null) return x.value;
            }
            return 0;
        }
        function choose(x: Item | null | undefined, flag: boolean): number {
            return typeof x !== "undefined" && flag
                ? x === null ? 0 : x.value
                : 0;
        }
    "#);
}

#[test]
fn typeof_guard_distinguishes_test_writes_from_exiting_body_writes() {
    lower(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined): number {
            if (typeof x === "undefined") { x = undefined; return 0; }
            if (x === null) return 1;
            return x.value;
        }
    "#);

    let stale_nested_logical = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined): number {
            if ((typeof x !== "undefined" && (x = undefined, true))
                && (x !== null && x.value > 0)) return 1;
            return 0;
        }
    "#).unwrap();
    assert!(lower_module(&stale_nested_logical).is_err());

    lower(r#"
        type Item = { value: number };
        function read(x: Item | null | undefined, flag: boolean): number {
            if (typeof x !== "undefined" && flag) {
                if (x !== null) return x.value;
            }
            return 0;
        }
    "#);
}

#[test]
fn generator_resume_assignment_invalidates_completed_nullish_narrowing() {
    let stale_resume = thaw_parser::parse_typescript(r#"
        type Item = { value: number };
        function* read(x: Item | null | undefined): Generator<number, number, Item | null | undefined> {
            if (typeof x === "undefined") return 0;
            if (x === null) return 1;
            x = yield 2;
            return x.value;
        }
    "#).unwrap();
    assert!(lower_module(&stale_resume).is_err());

    lower(r#"
        type Item = { value: number };
        function* read(x: Item | null | undefined): Generator<number, number, Item | null | undefined> {
            x = yield 2;
            if (typeof x === "undefined") return 0;
            if (x === null) return 1;
            return x.value;
        }
    "#);
}

#[test]
fn function_typed_values_keep_runtime_undefined_checks() {
    // A later class-method read can carry undefined in the existing raw
    // Function pointer ABI. These source functions are already lowerable;
    // the test checks their HIR retains runtime checks before that producer
    // is enabled.
    let program = lower(r#"
        type Callback = (value: number) => number;
        function kind(callback: Callback): string { return typeof callback; }
        function maybeKind(callback: Callback | undefined): string { return typeof callback; }
        function branch(callback: Callback): number { if (callback) return 1; return 0; }
        function strict(callback: Callback): boolean { return callback === undefined; }
        function reverse(callback: Callback): boolean { return undefined === callback; }
        function loose(callback: Callback): boolean { return callback == null; }
        function main(): void {}
    "#);
    for name in ["kind", "maybeKind", "branch", "strict", "reverse", "loose"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        let lowered = format!("{:?}", function.body);
        assert!(lowered.contains("OptionalNone"), "{name}: {lowered}");
    }
}


#[test]
fn tagged_function_absence_is_observed_before_equality_fallbacks() {
    let program = lower(r#"
        type Callback = (value: number) => number;
        function strictOther(callback: Callback | undefined, number: number | undefined): boolean {
            return callback === number;
        }
        function strictReverse(number: number | undefined, callback: Callback | undefined): boolean {
            return number === callback;
        }
        function strictNull(callback: Callback | null, number: number | null): boolean {
            return callback === number;
        }
        function looseNull(callback: Callback | undefined, other: null): boolean {
            return callback == other;
        }
        function typeofNested(callback: (Callback | undefined) | null): string {
            return typeof callback;
        }
        function sparse(callbacks: Callback[], index: number): boolean {
            return callbacks[index] === undefined;
        }
        function replaceSparse(callbacks: Callback[], index: number, replacement: Callback): undefined {
            callbacks[index] = replacement;
            return undefined;
        }
        function sparseMutatingRhs(callbacks: Callback[], index: number, replacement: Callback): boolean {
            return callbacks[index] === replaceSparse(callbacks, index, replacement);
        }
        function reverseSparseMutation(callbacks: Callback[], index: number, replacement: Callback): boolean {
            return replaceSparse(callbacks, index, replacement) === callbacks[index];
        }
        function main(): void {}
    "#);
    for name in ["strictOther", "strictReverse", "strictNull", "looseNull", "typeofNested", "sparse", "sparseMutatingRhs", "reverseSparseMutation"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        let body = format!("{:?}", function.body);
        assert!(body.contains("Conditional"), "{name}: {body}");
        if name != "strictNull" {
            assert!(body.contains("OptionalNone"), "{name}: {body}");
        } else {
            assert!(body.contains("NullableIsNone"), "{name}: {body}");
        }
        if name == "strictOther" || name == "strictReverse" {
            assert!(body.contains("OptionalIsNone"), "{name}: {body}");
            assert!(body.contains("__thaw_strict_left_status_"), "{name}: {body}");
            assert!(body.contains("__thaw_strict_right_status_"), "{name}: {body}");
        }
        if name == "sparse" || name == "sparseMutatingRhs" {
            assert!(body.contains("__thaw_array_index_state"), "{name}: {body}");
            assert!(body.contains("__thaw_index_missing_"), "{name}: {body}");
        }
        if name == "sparseMutatingRhs" {
            // The missing/present snapshot is bound by an outer lambda,
            // before the RHS call that can fill the formerly sparse slot.
            assert!(body.contains("replaceSparse"), "{name}: {body}");
            assert!(body.contains("__thaw_index_undefined_"), "{name}: {body}");
        }
        if name == "reverseSparseMutation" {
            assert!(body.contains("replaceSparse"), "{name}: {body}");
        }
    }
}

#[test]
fn tagged_function_present_leaves_keep_native_identity_and_ordered_dynamic_comparison() {
    let program = lower(r#"
        type Callback = (value: number) => number;
        function looseMixed(callback: Callback | undefined, number: number | undefined): boolean {
            return callback == number;
        }
        function looseReverse(number: number | undefined, callback: Callback | undefined): boolean {
            return number == callback;
        }
        function unionMixed(value: Callback | number | undefined, number: number | undefined): boolean {
            return value == number;
        }
        function strictLive(callback: Callback, value: any): boolean {
            return callback === value;
        }
        function symbolMixed(callback: Callback | undefined, value: symbol | undefined): boolean {
            return callback == value;
        }
        function bigintMixed(callback: Callback | undefined, value: bigint | undefined): boolean {
            return callback == value;
        }
        function dictionaryStrict(callback: Callback, value: Record<string, any>): boolean {
            return callback === value;
        }
        function dictionaryLoose(callback: Callback | undefined, value: Record<string, any>): boolean {
            return callback == value;
        }
        type RestCallback = (head: number, ...rest: number[]) => number;
        function callableMixed(callback: RestCallback | undefined, value: number | undefined): boolean {
            return callback == value;
        }
        function main(): void {}
    "#);
    for name in ["looseMixed", "looseReverse", "unionMixed", "symbolMixed", "bigintMixed", "callableMixed"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        let body = format!("{:?}", function.body);
        assert!(body.contains("__thaw_native_loose_equal"), "{name}: {body}");
        assert!(body.contains("__thaw_loose_left_status_"), "{name}: {body}");
        assert!(body.contains("__thaw_loose_right_status_"), "{name}: {body}");
    }
    let strict = program.functions.iter().find(|function| function.name == "strictLive").unwrap();
    let body = format!("{:?}", strict.body);
    assert!(body.contains("__thaw_strict_equal_dynamic"), "{body}");
    assert!(body.contains("callDynamicValueMixed"), "{body}");
    assert!(body.contains("registerNativeCallback"), "{body}");
    assert!(body.contains("JsValueAsJson"), "{body}");
    assert!(body.contains("F64(3.0)"), "native Function side must retain the trusted identity flag: {body}");
    for (name, comparison) in [("dictionaryStrict", "__thaw_strict_equal_dynamic"),
        ("dictionaryLoose", "__thaw_native_loose_equal")] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        let body = format!("{:?}", function.body);
        assert!(body.contains("__thaw_json_is_undefined"), "{name}: {body}");
        assert!(body.contains(comparison), "{name}: {body}");
        assert!(body.contains("TypedClosure(Json"), "{name}: {body}");
    }
    let callable = program.functions.iter().find(|function| function.name == "callableMixed").unwrap();
    let body = format!("{:?}", callable.body);
    assert!(body.contains("registerNativeCallback"), "{body}");
    assert!(body.contains("JsValueAsJson"), "{body}");
}

#[test]
fn function_dynamic_conversion_keeps_raw_undefined_before_registration() {
    let program = lower(r#"
        function asJson(callback: (value: number) => number): any { return callback; }
        function asHandle(callback: (value: number) => number): JsValue { return callback; }
        function asCallableJson(callback: (head: number, ...rest: number[]) => number): any {
            return callback;
        }
        function main(): void {}
    "#);
    for name in ["asJson", "asCallableJson"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        let body = format!("{:?}", function.body);
        assert!(body.contains("__thaw_function_json_source_"), "{name}: {body}");
        assert!(body.contains("OptionalNone"), "{name}: {body}");
        assert!(body.contains("Conditional"), "{name}: {body}");
        assert!(body.contains("registerNativeCallback"), "{name}: {body}");
    }
    let handle = program.functions.iter().find(|function| function.name == "asHandle").unwrap();
    assert!(format!("{:?}", handle.body).contains("registerNativeCallback"));
}

#[test]
fn dynamic_symbol_rejects_nonascii_hex_without_slicing_codepoints() {
    assert!(dynamic_symbol("__thaw_typed_js_aéa").is_none());
    assert!(dynamic_symbol("__thaw_typed_napi_zz").is_none());
    assert_eq!(dynamic_symbol("__thaw_typed_js_666f6f__arity_1"),
        Some((DynamicBackend::QuickJs, "foo".into())));
}

// Receiver-generic inference controls for the reviewed r1 prerequisite. These
// tests deliberately stop at metadata and actual-call type inference; they do
// not claim a receiver-aware specialization ABI or runtime function value.
fn receiver_pattern_inference_function(
    module: &swc_ecma_ast::Module,
) -> &swc_ecma_ast::FnDecl {
    module
        .body
        .iter()
        .find_map(|item| match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) => Some(function),
            _ => None,
        })
        .expect("parsed source contains a function declaration")
}

fn receiver_pattern_inference_this_type(source: &str) -> TsType {
    let module = thaw_parser::parse_typescript(source).expect("parse receiver annotation fixture");
    let function = receiver_pattern_inference_function(&module);
    function
        .function
        .this_param
        .as_ref()
        .and_then(|parameter| parameter.type_ann.as_ref())
        .expect("function has an annotated this parameter")
        .type_ann
        .as_ref()
        .clone()
}

#[allow(clippy::type_complexity)]
fn receiver_pattern_inference_type_parameters(
    source: &str,
) -> Vec<(Option<Box<TsType>>, Option<Box<TsType>>)> {
    let module = thaw_parser::parse_typescript(source).expect("parse generic type fixture");
    let function = receiver_pattern_inference_function(&module);
    function
        .function
        .type_params
        .as_ref()
        .expect("function has generic type parameters")
        .params
        .iter()
        .map(|parameter| (parameter.constraint.clone(), parameter.default.clone()))
        .collect()
}

fn receiver_pattern_inference_type_from_return(source: &str) -> TsType {
    let module = thaw_parser::parse_typescript(source).expect("parse explicit type fixture");
    let function = receiver_pattern_inference_function(&module);
    function
        .function
        .return_type
        .as_ref()
        .expect("function has an annotated return type")
        .type_ann
        .as_ref()
        .clone()
}

fn receiver_pattern_inference_signature(
    uses_this: bool,
    generic_this_pattern: Option<GenericReceiverPattern>,
    type_parameters: &[&str],
    visible_patterns: Vec<GenericTypePattern>,
) -> FnSignature {
    let physical_parameter_count = visible_patterns.len() + usize::from(uses_this);
    FnSignature {
        params: vec![HirType::Dynamic; physical_parameter_count],
        variadic: None,
        native_rest: None,
        abstract_class_constructor: false,
        ret: HirType::Dynamic,
        is_async: false,
        returns_sparse_array: false,
        uses_this,
        is_extern: false,
        source_range: (0, 0),
        accessor_owner: None,
        generic_type_params: type_parameters.iter().map(|name| (*name).to_string()).collect(),
        generic_type_constraints: vec![None; type_parameters.len()],
        generic_type_defaults: vec![None; type_parameters.len()],
        generic_param_patterns: visible_patterns.clone(),
        generic_param_optional: vec![false; visible_patterns.len()],
        generic_return_type: None,
        type_predicate: None,
        generic_return_pattern: None,
        generic_this_pattern,
    }
}

fn receiver_pattern_inference_apply_type_parameters(
    signature: &mut FnSignature,
    source: &str,
) {
    let parameters = receiver_pattern_inference_type_parameters(source);
    assert_eq!(parameters.len(), signature.generic_type_params.len());
    signature.generic_type_constraints = parameters
        .iter()
        .map(|(constraint, _)| constraint.clone())
        .collect();
    signature.generic_type_defaults = parameters
        .iter()
        .map(|(_, default)| default.clone())
        .collect();
}

#[test]
fn receiver_pattern_inference_parsed_annotations_keep_pattern_and_legacy_states() {
    let mut substitutions = HashMap::new();
    substitutions.insert("T".to_string(), GenericTypePattern::Variable("T".to_string()));
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();

    let variable = receiver_pattern_inference_this_type(
        "declare function receiver_pattern_inference_pick<T>(this: T): T;",
    );
    assert!(matches!(
        generic_receiver_pattern_from_annotation(
            &variable,
            &substitutions,
            &interfaces,
            &generic_interfaces,
        ),
        GenericReceiverPattern::Pattern(GenericTypePattern::Variable(name)) if name == "T"
    ));

    let concrete = receiver_pattern_inference_this_type(
        "declare function receiver_pattern_inference_concrete<T>(this: number, value: T): T;",
    );
    assert!(matches!(
        generic_receiver_pattern_from_annotation(
            &concrete,
            &substitutions,
            &interfaces,
            &generic_interfaces,
        ),
        GenericReceiverPattern::Pattern(GenericTypePattern::Concrete(HirType::F64))
    ));

    let union = receiver_pattern_inference_this_type(
        "declare function receiver_pattern_inference_union<T>(this: T | number, value: T): T;",
    );
    assert!(matches!(
        generic_receiver_pattern_from_annotation(
            &union,
            &substitutions,
            &interfaces,
            &generic_interfaces,
        ),
        GenericReceiverPattern::LegacyUnmatched
    ));
}

#[test]
fn receiver_pattern_inference_declarations_remain_accepted_without_calls() {
    let module = thaw_parser::parse_typescript(
        r#"
        declare function receiver_pattern_inference_pick<T>(this: T): T;
        declare function receiver_pattern_inference_concrete<T>(this: number, value: T): T;
        declare function receiver_pattern_inference_union<T>(this: T | number, value: T): T;
        declare function receiver_pattern_inference_id<T>(value: T): T;
        function receiver_pattern_inference_main(): void {}
        "#,
    )
    .expect("parse accepted generic-this declaration fixture");
    let program = lower_module(&module)
        .expect("generic this annotations accepted by physical type resolution remain accepted");
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == "receiver_pattern_inference_main"));
}

#[test]
fn receiver_pattern_inference_source_negative_and_shape_controls() {
    // The two unbound names in the source-order case fail during type
    // extraction. Seeing the receiver name first checks lowering order only;
    // this is not a runtime side-effect-order claim.
    for (source, expected) in [
        (
            r#"declare function receiver_pattern_inference_same<T>(this: T, value: T): T;
               function receiver_pattern_inference_main(): void {
                   receiver_pattern_inference_same.call(1, "visible");
               }"#,
            "conflicting call-site types F64 and Str",
        ),
        (
            r#"declare function receiver_pattern_inference_apply<T>(this: T, value: T): T;
               function receiver_pattern_inference_main(): void {
                   receiver_pattern_inference_apply.apply(1, ["visible"]);
               }"#,
            "conflicting call-site types F64 and Str",
        ),
        (
            r#"declare function receiver_pattern_inference_number<T>(this: number, value: T): T;
               function receiver_pattern_inference_main(): void {
                   receiver_pattern_inference_number.call("wrong", 1);
               }"#,
            "incompatible with parameter pattern",
        ),
        (
            r#"declare function receiver_pattern_inference_needs_this<T>(this: T): T;
               function receiver_pattern_inference_main(): void {
                   receiver_pattern_inference_needs_this.call();
               }"#,
            "thisArg",
        ),
        (
            r#"declare function receiver_pattern_inference_optional<T>(this: T, required: T, optional?: T): T;
               function receiver_pattern_inference_main(): void {
                   receiver_pattern_inference_optional.call("receiver");
               }"#,
            "expects",
        ),
        (
            r#"declare function receiver_pattern_inference_optional_overrun<T>(this: T, value?: T): T;
               function receiver_pattern_inference_main(): void {
                   receiver_pattern_inference_optional_overrun.call("receiver", 1, 2);
               }"#,
            "expects 2 argument(s), got 3",
        ),
        (
            r#"function receiver_pattern_inference_rest<T>(this: T, value: T, ...values: T[]): T {
                   return value;
               }
               function receiver_pattern_inference_main(): void {
                   receiver_pattern_inference_rest.call(1, "visible", ...["tail"]);
               }"#,
            "conflicting call-site types F64 and Str",
        ),
        (
            r#"declare function receiver_pattern_inference_free<T>(left: T, right: T): T;
               function receiver_pattern_inference_main(): void {
                   receiver_pattern_inference_free(1, "different");
               }"#,
            "conflicting call-site types",
        ),
        (
            r#"function receiver_pattern_inference_order<T>(this: T, value: T): T { return value; }
               function receiver_pattern_inference_main(): void {
                   receiver_pattern_inference_order.call(receiver_first_missing, visible_second_missing);
               }"#,
            "unknown variable `receiver_first_missing`",
        ),
        (
            r#"function receiver_pattern_inference_key<T, K extends keyof T>(this: T, key: K): T[K] {
                   return this[key];
               }
               function receiver_pattern_inference_main(): void {
                   receiver_pattern_inference_key.call({ name: "value" }, "missing");
               }"#,
            "indexed access key `missing` does not exist on the object type",
        ),
        (
            r#"function receiver_pattern_inference_non_arrow<T>(this: T, value: T): T { return value; }
               function receiver_pattern_inference_main(): void {
                   const fn = function<T>(this: T, value: T): T { return value; };
                   fn.call("receiver", 1);
               }"#,
            "conflicting call-site types",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).expect("parse call-shape fixture");
        let error = lower_module(&module).expect_err("fixture records a negative control");
        assert!(error.contains(expected), "expected {expected:?}, got {error}");
    }
}

#[test]
fn receiver_pattern_inference_physical_and_split_modes_match_receiver_first() {
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let receiver_pattern = GenericTypePattern::Variable("T".to_string());
    let mut signature = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::Pattern(receiver_pattern.clone())),
        &["T"],
        vec![GenericTypePattern::Variable("T".to_string())],
    );
    let receiver_only = [HirType::F64];
    let receiver_only_signature = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::Pattern(receiver_pattern.clone())),
        &["T"],
        vec![],
    );
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &receiver_only_signature,
            GenericMatchActuals::Physical(&receiver_only),
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::F64]
    );
    let physical_agreement = [HirType::F64, HirType::F64];
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &signature,
            GenericMatchActuals::Physical(&physical_agreement),
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::F64]
    );

    let receiver = HirType::Str;
    let visible = [HirType::Str];
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &signature,
            GenericMatchActuals::ReceiverAndVisible {
                receiver: Some(&receiver),
                visible: &visible,
            },
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::Str]
    );

    let physical_conflict = [HirType::Str, HirType::F64];
    let error = infer_generic_type_tuple_for_call(
        &signature,
        GenericMatchActuals::Physical(&physical_conflict),
        &interfaces,
        &generic_interfaces,
        None,
    )
    .unwrap_err();
    assert!(error.contains("conflicting call-site types"), "{error}");

    signature.generic_this_pattern = Some(GenericReceiverPattern::Pattern(
        GenericTypePattern::Concrete(HirType::F64),
    ));
    let concrete_mismatch = [HirType::Str, HirType::Str];
    let error = infer_generic_type_tuple_for_call(
        &signature,
        GenericMatchActuals::Physical(&concrete_mismatch),
        &interfaces,
        &generic_interfaces,
        None,
    )
    .unwrap_err();
    assert!(error.contains("incompatible with parameter pattern"), "{error}");
}

#[test]
fn receiver_pattern_inference_synthetic_and_contextual_modes_stay_separate() {
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let synthetic_pattern = GenericTypePattern::Variable("R".to_string());
    let mut signature = receiver_pattern_inference_signature(
        false,
        None,
        &["R", "T"],
        vec![GenericTypePattern::Variable("T".to_string())],
    );
    signature.params.clear();
    assert!(signature.params.is_empty());
    assert!(!signature.uses_this);
    assert!(signature.generic_this_pattern.is_none());
    let receiver = HirType::Str;
    let visible = [HirType::F64];
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &signature,
            GenericMatchActuals::SyntheticReceiverAndVisible {
                pattern: &synthetic_pattern,
                receiver: &receiver,
                visible: &visible,
            },
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::Str, HirType::F64]
    );

    let synthetic_receiver_only = receiver_pattern_inference_signature(
        false,
        None,
        &["T"],
        vec![],
    );
    let undefined_receiver = HirType::Undefined;
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &synthetic_receiver_only,
            GenericMatchActuals::SyntheticReceiverAndVisible {
                pattern: &GenericTypePattern::Variable("T".to_string()),
                receiver: &undefined_receiver,
                visible: &[],
            },
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::Undefined]
    );

    let mut contextual_signature = receiver_pattern_inference_signature(
        true,
        None,
        &["T"],
        vec![GenericTypePattern::Variable("T".to_string())],
    );
    assert_eq!(contextual_signature.params.len(), 2);
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &contextual_signature,
            GenericMatchActuals::ContextualVisible(&visible),
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::F64]
    );
    contextual_signature.generic_return_pattern =
        Some(GenericTypePattern::Variable("T".to_string()));
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &contextual_signature,
            GenericMatchActuals::ContextualVisible(&[]),
            &interfaces,
            &generic_interfaces,
            Some(&HirType::Bool),
        )
        .unwrap(),
        vec![HirType::Bool]
    );

    let explicit_number = Box::new(receiver_pattern_inference_type_from_return(
        "declare function receiver_pattern_inference_number(): number;",
    ));
    let mut explicit_contextual = receiver_pattern_inference_signature(
        true,
        None,
        &["T"],
        vec![GenericTypePattern::Variable("T".to_string())],
    );
    explicit_contextual.generic_this_pattern = None;
    let contextual_actual = [HirType::Str];
    let error = resolve_explicit_generic_type_tuple(
        &explicit_contextual,
        &[explicit_number],
        GenericMatchActuals::ContextualVisible(&contextual_actual),
        &interfaces,
        &generic_interfaces,
    )
    .unwrap_err();
    assert!(
        error.contains("argument infers T as Str, but explicit type is F64"),
        "{error}"
    );

    let mut explicit_synthetic = receiver_pattern_inference_signature(
        false,
        None,
        &["R", "T"],
        vec![GenericTypePattern::Variable("T".to_string())],
    );
    explicit_synthetic.params.clear();
    assert!(explicit_synthetic.params.is_empty());
    assert!(!explicit_synthetic.uses_this);
    assert!(explicit_synthetic.generic_this_pattern.is_none());
    let explicit_number = Box::new(receiver_pattern_inference_type_from_return(
        "declare function receiver_pattern_inference_number(): number;",
    ));
    let explicit_receiver = HirType::Str;
    let explicit_visible = [HirType::F64];
    let error = resolve_explicit_generic_type_tuple(
        &explicit_synthetic,
        &[explicit_number.clone(), explicit_number],
        GenericMatchActuals::SyntheticReceiverAndVisible {
            pattern: &synthetic_pattern,
            receiver: &explicit_receiver,
            visible: &explicit_visible,
        },
        &interfaces,
        &generic_interfaces,
    )
    .unwrap_err();
    assert!(
        error.contains("argument infers R as Str, but explicit type is F64"),
        "{error}"
    );
}

#[test]
fn receiver_pattern_inference_distinguishes_absent_undefined_and_legacy_receivers() {
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let signature = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::Pattern(GenericTypePattern::Variable(
            "T".to_string(),
        ))),
        &["T"],
        vec![],
    );
    let undefined = HirType::Undefined;
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &signature,
            GenericMatchActuals::ReceiverAndVisible {
                receiver: Some(&undefined),
                visible: &[],
            },
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::Undefined]
    );
    let error = infer_generic_type_tuple_for_call(
        &signature,
        GenericMatchActuals::ReceiverAndVisible {
            receiver: None,
            visible: &[],
        },
        &interfaces,
        &generic_interfaces,
        None,
    )
    .unwrap_err();
    assert!(error.contains("receiver") || error.contains("thisArg"), "{error}");

    let mut missing_metadata = signature.clone();
    missing_metadata.generic_this_pattern = None;
    let error = infer_generic_type_tuple_for_call(
        &missing_metadata,
        GenericMatchActuals::Physical(&[HirType::F64]),
        &interfaces,
        &generic_interfaces,
        None,
    )
    .unwrap_err();
    assert!(error.contains("metadata") || error.contains("receiver"), "{error}");

    let legacy = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::LegacyUnmatched),
        &["T"],
        vec![GenericTypePattern::Variable("T".to_string())],
    );
    let legacy_actuals = [HirType::Bool, HirType::Str];
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &legacy,
            GenericMatchActuals::Physical(&legacy_actuals),
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::Str]
    );
    let error = infer_generic_type_tuple_for_call(
        &legacy,
        GenericMatchActuals::Physical(&[]),
        &interfaces,
        &generic_interfaces,
        None,
    )
    .unwrap_err();
    assert!(error.contains("receiver") || error.contains("thisArg"), "{error}");
}

#[test]
fn receiver_pattern_inference_optional_rest_indices_and_key_literals_stay_visible_only() {
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let mut optional = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::Pattern(GenericTypePattern::Variable(
            "T".to_string(),
        ))),
        &["T"],
        vec![GenericTypePattern::Variable("T".to_string())],
    );
    optional.generic_param_optional = vec![true];
    let receiver = HirType::Str;
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &optional,
            GenericMatchActuals::ReceiverAndVisible {
                receiver: Some(&receiver),
                visible: &[],
            },
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::Str]
    );

    let rest = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::Pattern(GenericTypePattern::Variable(
            "R".to_string(),
        ))),
        &["R", "T"],
        vec![GenericTypePattern::Array(Box::new(
            GenericTypePattern::Variable("T".to_string()),
        ))],
    );
    let physical = [HirType::Bool, HirType::Array(Box::new(HirType::F64))];
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &rest,
            GenericMatchActuals::Physical(&physical),
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::Bool, HirType::F64]
    );
    assert!(matches!(
        generic_pattern_for_physical_argument(&rest, 0),
        Some(GenericTypePattern::Variable(name)) if name == "R"
    ));
    assert!(matches!(
        generic_pattern_for_physical_argument(&rest, 1),
        Some(GenericTypePattern::Array(inner))
            if matches!(inner.as_ref(), GenericTypePattern::Variable(name) if name == "T")
    ));
    assert!(generic_pattern_for_physical_argument(&rest, 2).is_none());

    let mut key_signature = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::Pattern(GenericTypePattern::Variable(
            "T".to_string(),
        ))),
        &["T", "Key"],
        vec![GenericTypePattern::Variable("Key".to_string())],
    );
    receiver_pattern_inference_apply_type_parameters(
        &mut key_signature,
        "declare function receiver_pattern_inference_key<T, Key extends keyof T>(this: T, key: Key): T[Key];",
    );
    let literal_key = HirType::StrLiteral("name".to_string());
    let key_actuals = [HirType::Object(vec![("name".to_string(), HirType::Str)]), literal_key.clone()];
    assert!(matches!(
        generic_pattern_for_physical_argument(&key_signature, 1),
        Some(GenericTypePattern::Variable(name)) if name == "Key"
    ));
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &key_signature,
            GenericMatchActuals::Physical(&key_actuals),
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![key_actuals[0].clone(), literal_key]
    );

    let no_this = receiver_pattern_inference_signature(
        false,
        None,
        &["T"],
        vec![GenericTypePattern::Variable("T".to_string())],
    );
    assert!(matches!(
        generic_pattern_for_physical_argument(&no_this, 0),
        Some(GenericTypePattern::Variable(name)) if name == "T"
    ));
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &no_this,
            GenericMatchActuals::Physical(&[HirType::Str]),
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::Str]
    );
    assert_eq!(
        infer_generic_type_tuple(
            &no_this,
            &[HirType::F64],
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::F64]
    );
    let legacy_receiver_index = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::LegacyUnmatched),
        &["T", "U"],
        vec![
            GenericTypePattern::Variable("T".to_string()),
            GenericTypePattern::Variable("U".to_string()),
        ],
    );
    assert!(generic_pattern_for_physical_argument(&legacy_receiver_index, 0).is_none());
    assert!(matches!(
        generic_pattern_for_physical_argument(&legacy_receiver_index, 1),
        Some(GenericTypePattern::Variable(name)) if name == "T"
    ));
    assert!(matches!(
        generic_pattern_for_physical_argument(&legacy_receiver_index, 2),
        Some(GenericTypePattern::Variable(name)) if name == "U"
    ));
    assert!(generic_pattern_for_physical_argument(&legacy_receiver_index, 3).is_none());
}

#[test]
fn receiver_pattern_inference_implicit_fallbacks_follow_actual_match_order() {
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let receiver = HirType::Bool;
    let mut defaulted = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::LegacyUnmatched),
        &["T"],
        vec![],
    );
    receiver_pattern_inference_apply_type_parameters(
        &mut defaulted,
        "declare function receiver_pattern_inference_default<T = string>(): T;",
    );
    defaulted.generic_return_pattern = Some(GenericTypePattern::Variable("T".to_string()));
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &defaulted,
            GenericMatchActuals::ReceiverAndVisible {
                receiver: Some(&receiver),
                visible: &[],
            },
            &interfaces,
            &generic_interfaces,
            Some(&HirType::F64),
        )
        .unwrap(),
        vec![HirType::F64]
    );
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &defaulted,
            GenericMatchActuals::ReceiverAndVisible {
                receiver: Some(&receiver),
                visible: &[],
            },
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::Str]
    );

    let mut constrained = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::LegacyUnmatched),
        &["T"],
        vec![],
    );
    receiver_pattern_inference_apply_type_parameters(
        &mut constrained,
        "declare function receiver_pattern_inference_constraint<T extends number>(): T;",
    );
    constrained.generic_return_pattern = Some(GenericTypePattern::Variable("T".to_string()));
    let error = infer_generic_type_tuple_for_call(
        &constrained,
        GenericMatchActuals::ReceiverAndVisible {
            receiver: Some(&receiver),
            visible: &[],
        },
        &interfaces,
        &generic_interfaces,
        Some(&HirType::Str),
    )
    .unwrap_err();
    assert!(error.contains("does not satisfy constraint"), "{error}");
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &constrained,
            GenericMatchActuals::ReceiverAndVisible {
                receiver: Some(&receiver),
                visible: &[],
            },
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::F64]
    );

    let mut extern_fallback = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::LegacyUnmatched),
        &["T"],
        vec![],
    );
    extern_fallback.is_extern = true;
    assert_eq!(
        infer_generic_type_tuple_for_call(
            &extern_fallback,
            GenericMatchActuals::ReceiverAndVisible {
                receiver: Some(&receiver),
                visible: &[],
            },
            &interfaces,
            &generic_interfaces,
            None,
        )
        .unwrap(),
        vec![HirType::Dynamic]
    );

    let mut conflicting = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::Pattern(GenericTypePattern::Variable(
            "T".to_string(),
        ))),
        &["T"],
        vec![GenericTypePattern::Variable("T".to_string())],
    );
    receiver_pattern_inference_apply_type_parameters(
        &mut conflicting,
        "declare function receiver_pattern_inference_default<T extends number = number>(): T;",
    );
    conflicting.generic_return_pattern = Some(GenericTypePattern::Variable("T".to_string()));
    conflicting.is_extern = true;
    let actuals = [HirType::Str, HirType::F64];
    let error = infer_generic_type_tuple_for_call(
        &conflicting,
        GenericMatchActuals::Physical(&actuals),
        &interfaces,
        &generic_interfaces,
        Some(&HirType::Bool),
    )
    .unwrap_err();
    assert!(error.contains("conflicting call-site types"), "{error}");
}

#[test]
fn receiver_pattern_inference_explicit_constraints_precede_actual_mismatch() {
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let mut constrained = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::Pattern(GenericTypePattern::Concrete(
            HirType::F64,
        ))),
        &["T"],
        vec![],
    );
    receiver_pattern_inference_apply_type_parameters(
        &mut constrained,
        "declare function receiver_pattern_inference_constraint<T extends number>(): T;",
    );
    let explicit_string = Box::new(receiver_pattern_inference_type_from_return(
        "declare function receiver_pattern_inference_string(): string;",
    ));
    let actual_receiver = [HirType::Str];
    let error = resolve_explicit_generic_type_tuple(
        &constrained,
        std::slice::from_ref(&explicit_string),
        GenericMatchActuals::Physical(&actual_receiver),
        &interfaces,
        &generic_interfaces,
    )
    .unwrap_err();
    assert!(
        error.contains("explicit type Str does not satisfy constraint F64"),
        "{error}"
    );
    assert!(!error.contains("argument infers"), "{error}");
    assert!(
        !error.contains("incompatible with parameter pattern"),
        "{error}"
    );

    let mut valid_constraint = receiver_pattern_inference_signature(
        true,
        Some(GenericReceiverPattern::Pattern(GenericTypePattern::Variable(
            "T".to_string(),
        ))),
        &["T"],
        vec![],
    );
    receiver_pattern_inference_apply_type_parameters(
        &mut valid_constraint,
        "declare function receiver_pattern_inference_constraint<T extends string>(): T;",
    );
    let receiver_conflict = [HirType::F64];
    let error = resolve_explicit_generic_type_tuple(
        &valid_constraint,
        &[explicit_string],
        GenericMatchActuals::Physical(&receiver_conflict),
        &interfaces,
        &generic_interfaces,
    )
    .unwrap_err();
    assert!(
        error.contains("argument infers T as F64, but explicit type is Str"),
        "{error}"
    );
}

#[test]
fn receiver_pattern_inference_type_only_and_receiver_free_promise_controls() {
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let mut defaulted = receiver_pattern_inference_signature(
        true,
        None,
        &["T"],
        vec![GenericTypePattern::Variable("T".to_string())],
    );
    receiver_pattern_inference_apply_type_parameters(
        &mut defaulted,
        "declare function receiver_pattern_inference_default<T = string>(): T;",
    );
    assert_eq!(
        resolve_explicit_generic_type_tuple(
            &defaulted,
            &[],
            GenericMatchActuals::TypeOnly,
            &interfaces,
            &generic_interfaces,
        )
        .unwrap(),
        vec![HirType::Str]
    );

    let module = thaw_parser::parse_typescript(
        r#"
        function receiver_pattern_inference_id<T>(value: T): T { return value; }
        function receiver_pattern_inference_callback<T>(value: T): T { return value; }
        function receiver_pattern_inference_main(): void {
            const specialized = receiver_pattern_inference_id<number>;
            const pending = new Promise<string>((resolve) => resolve("ok"))
                .then(receiver_pattern_inference_callback);
        }
        "#,
    )
    .expect("parse receiver-free instantiation and named Promise callback fixture");
    lower_module(&module)
        .expect("receiver-free explicit instantiation and named contextual Promise callback remain controls");
}


// Source-only direct controls for Error constructor argument staging. These
// fixtures call the argument helper itself, never an Error constructor or
// held field builder. All executable gates remain unrun.
fn error_argument_staging_signature(ret: HirType) -> FnSignature {
    FnSignature {
        params: Vec::new(), variadic: None, native_rest: None,
        abstract_class_constructor: false, ret, is_async: false,
        returns_sparse_array: false, uses_this: false, is_extern: false,
        source_range: (0, 0), accessor_owner: None,
        generic_type_params: Vec::new(), generic_type_constraints: Vec::new(),
        generic_type_defaults: Vec::new(), generic_param_patterns: Vec::new(),
        generic_param_optional: Vec::new(), generic_this_pattern: None,
        generic_return_type: None, type_predicate: None, generic_return_pattern: None,
    }
}

fn error_argument_staging_signatures() -> HashMap<Symbol, FnSignature> {
    let object = HirType::Object(vec![("field".into(), HirType::F64)]);
    let carrier = crate::caught_exception_carrier_type();
    let mut signatures = HashMap::new();
    for (name, ret) in [
        ("error_stage_number", HirType::F64),
        ("error_stage_string", HirType::Str),
        ("error_stage_object", object.clone()),
        ("error_stage_bytes", HirType::Bytes),
        ("error_stage_json", HirType::Json),
        ("error_stage_js", HirType::JsValue),
        ("error_stage_carrier", carrier.clone()),
        ("error_stage_tuple", HirType::Tuple(vec![HirType::F64, HirType::Str])),
        ("error_stage_empty_tuple", HirType::Tuple(Vec::new())),
        ("error_stage_representation_tuple", HirType::Tuple(vec![
            object, HirType::Json, HirType::JsValue, carrier,
        ])),
        ("error_stage_array", HirType::Array(Box::new(HirType::F64))),
        ("error_stage_function", HirType::Function(Vec::new(), Box::new(HirType::F64))),
        ("error_stage_message", HirType::Str),
    ] {
        signatures.insert(name.into(), error_argument_staging_signature(ret));
    }
    signatures
}

fn with_error_argument_staging_lowerer<R>(
    signatures: &HashMap<Symbol, FnSignature>,
    f: impl FnOnce(&mut FnLowerer<'_>) -> R,
) -> R {
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let enum_values = EnumValues::new();
    let enum_reverse_values = EnumReverseValues::new();
    let mut lowerer = FnLowerer::new(signatures, &interfaces, &generic_interfaces,
        &enum_values, &enum_reverse_values, HirType::Void, None);
    f(&mut lowerer)
}

fn error_argument_staging_args(source: &str) -> Vec<swc_ecma_ast::ExprOrSpread> {
    let module = thaw_parser::parse_typescript(source).expect("parse argument staging fixture");
    assert_eq!(module.body.len(), 1);
    let ModuleItem::Stmt(Stmt::Expr(statement)) = &module.body[0] else {
        panic!("expected one expression statement");
    };
    let Expr::Call(call) = statement.expr.as_ref() else { panic!("expected call expression"); };
    call.args.clone()
}

fn error_argument_staging_call(name: &str) -> HirExpr {
    HirExpr::Call(Box::new(HirExpr::Var(name.into())), Vec::new())
}


fn assert_error_argument_staging_fresh_vars(
    lowerer: &FnLowerer<'_>,
    values: &[HirExpr],
    bindings: &[(Symbol, HirType, HirExpr)],
) {
    let names: std::collections::HashSet<_> = bindings.iter().map(|(name, _, _)| name.clone()).collect();
    assert_eq!(names.len(), bindings.len(), "staging names are fresh");
    let mut returned = std::collections::HashSet::new();
    for value in values {
        let HirExpr::Var(name) = value else { panic!("staged value must be a Var"); };
        assert!(returned.insert(name.clone()), "returned Vars must be distinct");
        let (_, expected, _) = bindings.iter().find(|(binding, _, _)| binding == name)
            .expect("returned Var has a staging binding");
        let actual = lowerer.infer_expr_type(value).unwrap();
        assert_eq!(&actual, expected, "returned Var has its registered type");
    }
}

fn error_argument_staging_array_read_expected(
    index_value: f64,
    receiver_name: &str,
    offset_name: &str,
) -> HirExpr {
    let array = HirType::Array(Box::new(HirType::F64));
    let optional = HirType::Optional(Box::new(HirType::F64));
    let receiver = HirExpr::Var(receiver_name.into());
    let offset = HirExpr::Var(offset_name.into());
    let absent = HirExpr::OptionalNone(HirType::F64);
    let present = HirExpr::OptionalSome(Box::new(HirExpr::TypedIndex(
        Box::new(receiver.clone()), Box::new(offset.clone()), HirType::F64,
    )), HirType::F64);
    let slot_state = HirExpr::Call(Box::new(HirExpr::Var("__thaw_array_index_state".into())),
        vec![receiver.clone(), offset.clone()]);
    let present_if_slot = HirExpr::Conditional(
        Box::new(HirExpr::BinOp(BinOp::EqEqEq, Box::new(slot_state),
            Box::new(HirExpr::Lit(HirLit::F64(1.0))))),
        Box::new(present), Box::new(absent.clone()), optional.clone());
    let present_if_in_bounds = HirExpr::Conditional(
        Box::new(HirExpr::BinOp(BinOp::Lt, Box::new(offset.clone()),
            Box::new(HirExpr::ArrayLen(Box::new(receiver.clone()))))),
        Box::new(present_if_slot), Box::new(absent.clone()), optional.clone());
    let present_if_nonnegative = HirExpr::Conditional(
        Box::new(HirExpr::BinOp(BinOp::GtEq, Box::new(offset.clone()),
            Box::new(HirExpr::Lit(HirLit::F64(0.0))))),
        Box::new(present_if_in_bounds), Box::new(absent.clone()), optional.clone());
    let present_if_integer = HirExpr::Conditional(
        Box::new(HirExpr::BinOp(BinOp::EqEqEq, Box::new(offset.clone()),
            Box::new(HirExpr::Call(Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                vec![offset])))),
        Box::new(present_if_nonnegative), Box::new(absent), optional.clone());
    let offset_wrapper = HirExpr::Call(Box::new(HirExpr::Lambda(
        vec![HirParam { name: receiver_name.into(), ty: array.clone() }],
        vec![HirParam { name: offset_name.into(), ty: HirType::F64 }],
        optional.clone(), Box::new(present_if_integer))),
        vec![HirExpr::Lit(HirLit::F64(index_value))]);
    HirExpr::Call(Box::new(HirExpr::Lambda(Vec::new(),
        vec![HirParam { name: receiver_name.into(), ty: array }],
        optional, Box::new(offset_wrapper))),
        vec![error_argument_staging_call("error_stage_array")])
}

#[test]
fn error_argument_staging_keeps_all_raw_values_once() {
    let signatures = error_argument_staging_signatures();
    let args = error_argument_staging_args("sink(error_stage_number(), error_stage_string(), error_stage_object(), error_stage_bytes(), error_stage_number(), error_stage_string())");
    with_error_argument_staging_lowerer(&signatures, |lowerer| {
        let (values, bindings) = lowerer.lower_error_constructor_arguments(&args, "SuppressedError").unwrap();
        let types = [HirType::F64, HirType::Str,
            HirType::Object(vec![("field".into(), HirType::F64)]),
            HirType::Array(Box::new(HirType::F64)), HirType::F64, HirType::Str];
        let calls = ["error_stage_number", "error_stage_string", "error_stage_object",
            "error_stage_bytes", "error_stage_number", "error_stage_string"];
        assert_eq!(values.len(), 6);
        assert_eq!(bindings.len(), 6);
        for i in 0..6 {
            assert_eq!(bindings[i].1, types[i], "inferred storage type {i}");
            assert_eq!(bindings[i].2, error_argument_staging_call(calls[i]));
            assert_eq!(values[i], HirExpr::Var(bindings[i].0.clone()));
        }
        assert!(values.iter().all(|value| matches!(value, HirExpr::Var(_))));
        assert_error_argument_staging_fresh_vars(lowerer, &values, &bindings);
    });
}

#[test]
fn error_argument_staging_flattens_literal_spreads_in_order() {
    let signatures = error_argument_staging_signatures();
    let args = error_argument_staging_args("sink(error_stage_number(), ...[error_stage_number(), 7, undefined, , error_stage_string()], ...[], ...[false, null], error_stage_string())");
    with_error_argument_staging_lowerer(&signatures, |lowerer| {
        let (values, bindings) = lowerer.lower_error_constructor_arguments(&args, "SuppressedError").unwrap();
        let types = [HirType::F64, HirType::F64, HirType::F64, HirType::Undefined,
            HirType::Undefined, HirType::Str, HirType::Bool, HirType::Null, HirType::Str];
        let inits = [error_argument_staging_call("error_stage_number"),
            error_argument_staging_call("error_stage_number"), HirExpr::Lit(HirLit::F64(7.0)),
            HirExpr::Lit(HirLit::Undefined), HirExpr::Lit(HirLit::ArrayHole),
            error_argument_staging_call("error_stage_string"), HirExpr::Lit(HirLit::Bool(false)),
            HirExpr::Lit(HirLit::Null), error_argument_staging_call("error_stage_string")];
        assert_eq!(values.len(), 9);
        assert_eq!(bindings.len(), 9);
        for i in 0..9 {
            assert_eq!(bindings[i].1, types[i]);
            assert_eq!(bindings[i].2, inits[i]);
            assert_eq!(values[i], HirExpr::Var(bindings[i].0.clone()));
        }
        assert_error_argument_staging_fresh_vars(lowerer, &values, &bindings);
    });
}

#[test]
fn error_argument_staging_snapshots_tuple_members_before_next_argument() {
    let signatures = error_argument_staging_signatures();
    let args = error_argument_staging_args("sink(...error_stage_tuple(), error_stage_number())");
    with_error_argument_staging_lowerer(&signatures, |lowerer| {
        let (values, bindings) = lowerer.lower_error_constructor_arguments(&args, "SuppressedError").unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(bindings.len(), 4);
        assert_eq!(bindings[0].1, HirType::Tuple(vec![HirType::F64, HirType::Str]));
        assert_eq!(bindings[0].2, error_argument_staging_call("error_stage_tuple"));
        for (i, ty) in [HirType::F64, HirType::Str].iter().enumerate() {
            assert_eq!(bindings[i + 1].1, *ty);
            assert_eq!(bindings[i + 1].2, HirExpr::TypedIndex(
                Box::new(HirExpr::Var(bindings[0].0.clone())),
                Box::new(HirExpr::Lit(HirLit::F64(i as f64))), ty.clone()));
            assert_eq!(values[i], HirExpr::Var(bindings[i + 1].0.clone()));
        }
        assert_eq!(bindings[3].1, HirType::F64);
        assert_eq!(bindings[3].2, error_argument_staging_call("error_stage_number"));
        assert_eq!(values[2], HirExpr::Var(bindings[3].0.clone()));
        assert!(values.iter().all(|value| matches!(value, HirExpr::Var(_))));
        assert_error_argument_staging_fresh_vars(lowerer, &values, &bindings);
    });
}

#[test]
fn error_argument_staging_evaluates_empty_tuple_and_extras() {
    let signatures = error_argument_staging_signatures();
    let args = error_argument_staging_args("sink(...error_stage_empty_tuple(), error_stage_number(), ...[error_stage_string()])");
    with_error_argument_staging_lowerer(&signatures, |lowerer| {
        let (values, bindings) = lowerer.lower_error_constructor_arguments(&args, "SuppressedError").unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(bindings.len(), 3);
        assert_eq!(bindings[0].1, HirType::Tuple(Vec::new()));
        assert_eq!(bindings[0].2, error_argument_staging_call("error_stage_empty_tuple"));
        assert_eq!(bindings[1].2, error_argument_staging_call("error_stage_number"));
        assert_eq!(bindings[2].2, error_argument_staging_call("error_stage_string"));
        let wrapped = lowerer.wrap_call_argument_bindings(values[0].clone(), &bindings).unwrap();
        let mut current = &wrapped;
        for (name, ty, source) in &bindings {
            let HirExpr::Call(callee, args) = current else { panic!("expected binding call"); };
            assert_eq!(args, &vec![source.clone()]);
            let HirExpr::Lambda(_, params, result_type, body) = callee.as_ref() else { panic!("expected lambda"); };
            assert_eq!(params.len(), 1);
            assert_eq!(&params[0].name, name);
            assert_eq!(&params[0].ty, ty);
            assert_eq!(result_type, &HirType::F64);
            current = body.as_ref();
        }
        assert_eq!(current, &values[0]);
    });
}

#[test]
fn error_argument_staging_preserves_reference_and_union_representations() {
    let signatures = error_argument_staging_signatures();
    let args = error_argument_staging_args("sink(error_stage_object(), error_stage_json(), error_stage_js(), error_stage_carrier(), ...error_stage_representation_tuple())");
    with_error_argument_staging_lowerer(&signatures, |lowerer| {
        let (values, bindings) = lowerer.lower_error_constructor_arguments(&args, "SuppressedError").unwrap();
        let object = HirType::Object(vec![("field".into(), HirType::F64)]);
        let carrier = crate::caught_exception_carrier_type();
        let tuple_type = HirType::Tuple(vec![object.clone(), HirType::Json, HirType::JsValue, carrier.clone()]);
        assert_eq!(values.len(), 8);
        assert_eq!(bindings.len(), 9);
        let raw_types = [object.clone(), HirType::Json, HirType::JsValue, carrier.clone()];
        let raw_calls = ["error_stage_object", "error_stage_json", "error_stage_js", "error_stage_carrier"];
        for i in 0..4 {
            assert_eq!(bindings[i].1, raw_types[i]);
            assert_eq!(bindings[i].2, error_argument_staging_call(raw_calls[i]));
            assert_eq!(values[i], HirExpr::Var(bindings[i].0.clone()));
        }
        assert_eq!(bindings[4].1, tuple_type);
        assert_eq!(bindings[4].2, error_argument_staging_call("error_stage_representation_tuple"));
        for (i, ty) in [object, HirType::Json, HirType::JsValue, carrier].iter().enumerate() {
            let slot = i + 5;
            assert_eq!(bindings[slot].1, *ty);
            assert_eq!(bindings[slot].2, HirExpr::TypedIndex(
                Box::new(HirExpr::Var(bindings[4].0.clone())),
                Box::new(HirExpr::Lit(HirLit::F64(i as f64))), ty.clone()));
            assert_eq!(values[i + 4], HirExpr::Var(bindings[slot].0.clone()));
        }
        assert!(values.iter().all(|value| matches!(value, HirExpr::Var(_))));
        assert_error_argument_staging_fresh_vars(lowerer, &values, &bindings);
    });
}

#[test]
fn error_argument_staging_wrapper_precedes_message_work() {
    let signatures = error_argument_staging_signatures();
    let args = error_argument_staging_args("sink(error_stage_number(), ...error_stage_tuple(), error_stage_string())");
    with_error_argument_staging_lowerer(&signatures, |lowerer| {
        let (values, mut bindings) = lowerer.lower_error_constructor_arguments(&args, "SuppressedError").unwrap();
        assert_eq!(values.len(), 4);
        assert_eq!(bindings.len(), 5);
        assert_eq!(bindings[0].1, HirType::F64);
        assert_eq!(bindings[0].2, error_argument_staging_call("error_stage_number"));
        assert_eq!(bindings[1].1, HirType::Tuple(vec![HirType::F64, HirType::Str]));
        assert_eq!(bindings[1].2, error_argument_staging_call("error_stage_tuple"));
        for (i, ty) in [HirType::F64, HirType::Str].iter().enumerate() {
            assert_eq!(bindings[i + 2].1, *ty);
            assert_eq!(bindings[i + 2].2, HirExpr::TypedIndex(
                Box::new(HirExpr::Var(bindings[1].0.clone())),
                Box::new(HirExpr::Lit(HirLit::F64(i as f64))), ty.clone()));
            assert_eq!(values[i + 1], HirExpr::Var(bindings[i + 2].0.clone()));
        }
        assert_eq!(bindings[4].1, HirType::Str);
        assert_eq!(bindings[4].2, error_argument_staging_call("error_stage_string"));
        assert_eq!(values[0], HirExpr::Var(bindings[0].0.clone()));
        assert_eq!(values[3], HirExpr::Var(bindings[4].0.clone()));
        assert_error_argument_staging_fresh_vars(lowerer, &values, &bindings);

        let message_name = "__test_message_work".to_string();
        let message = error_argument_staging_call("error_stage_message");
        lowerer.scope.insert(message_name.clone(), HirType::Str);
        bindings.push((message_name, HirType::Str, message.clone()));
        assert_eq!(bindings.len(), 6);
        assert_error_argument_staging_fresh_vars(lowerer, &values, &bindings);
        let wrapped = lowerer.wrap_call_argument_bindings(values[0].clone(), &bindings).unwrap();
        let mut current = &wrapped;
        for (name, ty, source) in &bindings {
            let HirExpr::Call(callee, args) = current else { panic!("message work missing from wrapper"); };
            assert_eq!(args, &vec![source.clone()]);
            let HirExpr::Lambda(_, params, result_type, body) = callee.as_ref() else { panic!("expected lambda"); };
            assert_eq!(params.len(), 1);
            assert_eq!(&params[0].name, name);
            assert_eq!(&params[0].ty, ty);
            assert_eq!(result_type, &HirType::F64);
            current = body.as_ref();
        }
        assert_eq!(current, &values[0]);
        assert_eq!(bindings.last().unwrap().2, message);
        for name in ["error_stage_number", "error_stage_tuple", "error_stage_string", "error_stage_message"] {
            assert_eq!(bindings.iter().filter(|(_, _, init)| *init == error_argument_staging_call(name)).count(), 1, "{name}");
        }
    });
}

#[test]
fn existing_native_spread_staging_mode_false_is_unchanged() {
    let signatures = error_argument_staging_signatures();
    let args = error_argument_staging_args("sink(error_stage_number(), ...[1, , 3], ...error_stage_tuple(), ...error_stage_empty_tuple(), ...[])");
    with_error_argument_staging_lowerer(&signatures, |lowerer| {
        let (values, bindings) = lowerer.lower_native_spread_values(&args, "native control").unwrap();
        assert_eq!(bindings.len(), 3);
        assert_eq!(bindings[0].1, HirType::F64);
        assert_eq!(bindings[0].2, error_argument_staging_call("error_stage_number"));
        assert_eq!(bindings[1].1, HirType::Tuple(vec![HirType::F64, HirType::Str]));
        assert_eq!(bindings[1].2, error_argument_staging_call("error_stage_tuple"));
        assert_eq!(bindings[2].1, HirType::Tuple(Vec::new()));
        assert_eq!(bindings[2].2, error_argument_staging_call("error_stage_empty_tuple"));
        assert_eq!(values, vec![
            HirExpr::Var(bindings[0].0.clone()),
            HirExpr::Lit(HirLit::F64(1.0)), HirExpr::Lit(HirLit::ArrayHole),
            HirExpr::Lit(HirLit::F64(3.0)),
            HirExpr::TypedIndex(Box::new(HirExpr::Var(bindings[1].0.clone())),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))), HirType::F64),
            HirExpr::TypedIndex(Box::new(HirExpr::Var(bindings[1].0.clone())),
                Box::new(HirExpr::Lit(HirLit::F64(1.0))), HirType::Str),
        ]);
    });

    let args = error_argument_staging_args("sink(...[], (n) => n, (s) => s)");
    with_error_argument_staging_lowerer(&signatures, |lowerer| {
        let number_fn = HirType::Function(vec![HirType::F64], Box::new(HirType::F64));
        let string_fn = HirType::Function(vec![HirType::Str], Box::new(HirType::Str));
        let expected = [HirType::Tuple(Vec::new()), number_fn.clone()];
        let (values, bindings) = lowerer.lower_native_spread_values_with_expected(
            &args, "native expected", &expected, Some(&string_fn)).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].1, number_fn);
        assert_eq!(bindings[0].2, HirExpr::Lambda(Vec::new(), vec![HirParam {
            name: "n".into(), ty: HirType::F64,
        }], HirType::F64, Box::new(HirExpr::Var("n".into()))));
        assert_eq!(bindings[1].1, string_fn);
        assert_eq!(bindings[1].2, HirExpr::Lambda(Vec::new(), vec![HirParam {
            name: "s".into(), ty: HirType::Str,
        }], HirType::Str, Box::new(HirExpr::Var("s".into()))));
        assert_eq!(values[0], HirExpr::Var(bindings[0].0.clone()));
        assert_eq!(values[1], HirExpr::Var(bindings[1].0.clone()));
        assert_error_argument_staging_fresh_vars(lowerer, &values, &bindings);
    });

    let args = error_argument_staging_args("sink(error_stage_array()[0], ...[error_stage_array()[1]])");
    with_error_argument_staging_lowerer(&signatures, |lowerer| {
        let (values, bindings) = lowerer.lower_native_spread_array_values(&args, "native array-read").unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(bindings.len(), 2);
        let optional = HirType::Optional(Box::new(HirType::F64));
        assert_eq!(bindings[0].1, optional);
        assert_eq!(bindings[1].1, optional);
        assert_eq!(bindings[0].0, "__thaw_native_arg_2");
        assert_eq!(bindings[1].0, "__thaw_native_arg_5");
        assert_eq!(bindings[0].2, error_argument_staging_array_read_expected(
            0.0, "__thaw_index_receiver_0", "__thaw_index_offset_1"));
        assert_eq!(bindings[1].2, error_argument_staging_array_read_expected(
            1.0, "__thaw_index_receiver_3", "__thaw_index_offset_4"));
        assert_eq!(values[0], HirExpr::Var(bindings[0].0.clone()));
        assert_eq!(values[1], HirExpr::Var(bindings[1].0.clone()));
        assert_error_argument_staging_fresh_vars(lowerer, &values, &bindings);

        let literal_args = error_argument_staging_args("sink(error_stage_number(), ...[2, ,])");
        let (literal_values, literal_bindings) = lowerer.lower_native_spread_array_values(
            &literal_args, "native array-read literal").unwrap();
        assert_eq!(literal_bindings.len(), 1);
        assert_eq!(literal_bindings[0].1, HirType::F64);
        assert_eq!(literal_bindings[0].2, error_argument_staging_call("error_stage_number"));
        assert_eq!(literal_values, vec![HirExpr::Var(literal_bindings[0].0.clone()),
            HirExpr::Lit(HirLit::F64(2.0)), HirExpr::Lit(HirLit::ArrayHole)]);
    });
}

#[test]
fn error_argument_staging_retains_existing_spread_admission() {
    let signatures = error_argument_staging_signatures();
    for (source, label, source_type) in [
        ("sink(...error_stage_array())", "array control", "Array(F64)"),
        ("sink(...error_stage_json())", "json control", "Json"),
        ("sink(...error_stage_js())", "js control", "JsValue"),
        ("sink(...error_stage_function())", "function control", "Function([], F64)"),
    ] {
        let args = error_argument_staging_args(source);
        let error_only = with_error_argument_staging_lowerer(&signatures, |lowerer|
            lowerer.lower_error_constructor_arguments(&args, label).unwrap_err());
        let generic = with_error_argument_staging_lowerer(&signatures, |lowerer|
            lowerer.lower_native_spread_values(&args, label).unwrap_err());
        let expected = format!("{label} spread source must have statically known tuple length, got {source_type}");
        assert_eq!(error_only, generic, "inherited admission for {source}");
        assert_eq!(error_only, expected, "baseline diagnostic for {source}");
    }
}
