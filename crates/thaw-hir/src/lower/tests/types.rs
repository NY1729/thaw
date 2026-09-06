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
    assert!(
        matches!(&pair.body[0], HirStmt::Return(Some(HirExpr::ObjectLit(fields))) if
        fields[0].0 == "first" && fields[1].0 == "second")
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
        "interface Base<T> { value: T } interface Child<T> extends Base<T> { value: T } function bad(value: Child<number>): void {}",
    )
    .unwrap();
    assert!(lower_module(&module)
        .unwrap_err()
        .contains("collides with an inherited field"));
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
    // Declared via the interface name, but the literal is still
    // reordered to the interface's field order (same machinery as
    // inline object type literals).
    assert_eq!(
        main.body[0],
        HirStmt::Let(
            "p".into(),
            point_ty,
            HirExpr::ObjectLit(vec![
                ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
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
        program.functions[0].body[0],
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
        program.functions[1].body[0],
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
        program.functions[0].body[0],
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
        program.functions[0].body[0],
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
            console.log(cache.size);
        }"#,
    );
    assert!(matches!(
        program.functions[0].body[0],
        HirStmt::Let(_, HirType::Map(_, _), _)
    ));

    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const byString: WeakMap<string, number> = new WeakMap<string, number>();
        }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("WeakMap/WeakSet keys must be"), "{error}");
}
