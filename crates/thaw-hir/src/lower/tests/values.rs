/// `const err: any = e` on a caught exception must keep the caught value's
/// concrete `Str` type, not coerce it to `Json`. Coercing turned
/// `err.name`/`err.message` into plain JSON key lookups on a JSON string
/// (`undefined`), losing the `__thaw_error_name`/`__thaw_error_message`
/// routing a `Str` value gets. Real trigger: `catch (e) { const err: any =
/// e; err.name }` from an rxjs rejection.
#[test]
fn an_any_annotated_copy_of_a_caught_error_keeps_the_string_error_type() {
    let program = lower(
        r#"
            function main(): void {
                try {
                    throw new Error("boom");
                } catch (e) {
                    const err: any = e;
                    console.log(err.name, err.message);
                }
            }
        "#,
    );
    let source = format!("{:?}", program.functions);
    assert!(
        source.contains("__thaw_error_name"),
        "`.name` on the caught-error copy should route to `__thaw_error_name`:\n{source}"
    );
    assert!(
        source.contains("__thaw_error_message"),
        "`.message` on the caught-error copy should route to `__thaw_error_message`:\n{source}"
    );
}

#[test]
fn expands_nested_top_level_object_and_array_destructuring() {
    let program = lower(
        r#"
            const { point: { x, y }, values: [first, second] } = {
                point: { x: 40, y: 2 },
                values: [20, 22]
            };
            function main(): void {
                console.log(x + y);
                console.log(first + second);
            }
        "#,
    );
    for name in ["x", "y", "first", "second"] {
        let global = program
            .globals
            .iter()
            .find(|global| global.name == name)
            .unwrap_or_else(|| panic!("missing destructured global `{name}`"));
        assert_eq!(global.ty, HirType::F64);
        assert!(!global.mutable);
    }
}

#[test]
fn expands_top_level_destructuring_defaults_and_array_rest() {
    let program = lower(
        r#"
            interface Config { fallback: number | undefined; }
            const { fallback = 42 }: Config = { fallback: undefined };
            const [head, ...tail] = [20, 10, 12];
            const { answer, ...metadata } = { answer: 42, label: "ready", code: 2 };
            function main(): void {
                console.log(fallback);
                console.log(head + tail[0] + tail[1]);
                console.log(metadata.label);
            }
        "#,
    );
    assert_eq!(
        program
            .globals
            .iter()
            .find(|global| global.name == "fallback")
            .unwrap()
            .ty,
        HirType::F64
    );
    assert_eq!(
        program
            .globals
            .iter()
            .find(|global| global.name == "tail")
            .unwrap()
            .ty,
        HirType::Array(Box::new(HirType::F64))
    );
    assert_eq!(
        program
            .globals
            .iter()
            .find(|global| global.name == "metadata")
            .unwrap()
            .ty,
        HirType::Object(vec![
            ("label".into(), HirType::Str),
            ("code".into(), HirType::F64),
        ])
    );
}

#[test]
fn dynamic_computed_object_reads_form_heterogeneous_unions() {
    let program = lower(
        r#"function read(key: string): number | string | null | undefined {
            const mixed = { value: 1, label: "one", empty: null, absent: undefined };
            return mixed[key];
        }"#,
    );
    let HirStmt::Return(Some(HirExpr::DynamicPropAccess(_, _, fields, result))) =
        &program.functions[0].body[1]
    else {
        panic!("expected a dynamic property union return");
    };
    assert_eq!(fields[0].1, HirType::F64);
    assert_eq!(fields[1].1, HirType::Str);
    assert_eq!(
        result,
        &HirType::Union(vec![
            HirType::F64,
            HirType::Str,
            HirType::Null,
            HirType::Undefined
        ])
    );
}

#[test]
fn dynamic_computed_object_reads_flatten_tagged_heterogeneous_fields() {
    let program = lower(
        r#"function read(
            mixed: { value: number | undefined; label: string },
            key: string
        ): void { console.log(mixed[key]); }"#,
    );
    assert!(matches!(
        &program.functions[0].body[0],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(
                &args[0],
                HirExpr::DynamicPropAccess(
                    _, _, _, HirType::Union(members)
                ) if members == &[HirType::F64, HirType::Undefined, HirType::Str]
            )
    ));
}

#[test]
fn dynamic_computed_object_reads_flatten_uniform_tagged_fields() {
    let program = lower(
        r#"function read(
            optional: { a: number | undefined; b: number | undefined },
            nullable: { a: number | null; b: number | null },
            nullish: { a: number | null | undefined; b: number | null | undefined },
            key: string
        ): void {
            console.log(optional[key]);
            console.log(nullable[key]);
            console.log(nullish[key]);
        }"#,
    );
    let body = &program.functions[0].body;
    assert!(matches!(
        &body[0],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::DynamicPropAccess(_, _, _, HirType::Optional(inner)) if inner.as_ref() == &HirType::F64)
    ));
    assert!(matches!(
        &body[1],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::DynamicPropAccess(_, _, _, HirType::Nullish(inner)) if inner.as_ref() == &HirType::F64)
    ));
    assert!(matches!(
        &body[2],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::DynamicPropAccess(_, _, _, HirType::Nullish(inner)) if inner.as_ref() == &HirType::F64)
    ));
}

#[test]
fn lowers_numeric_and_string_enum_members_declared_after_functions() {
    let program = lower(
        r#"function value(direction: Direction): number {
            return direction + Direction.Next;
        }
        function main(): void {
            console.log(value(Direction.None));
            console.log(Direction["Mask"]);
            console.log(Label.Alias);
        }
        enum Direction { None, Up = 4, Next = Up + 2, Mask = 1 << 3 }
        enum Label { Ready = "ready", Alias = Ready }"#,
    );
    let value = program
        .functions
        .iter()
        .find(|function| function.name == "value")
        .unwrap();
    assert_eq!(value.params[0].ty, HirType::F64);
    assert!(matches!(
        &value.body[0],
        HirStmt::Return(Some(HirExpr::BinOp(BinOp::Add, _, right)))
            if matches!(right.as_ref(), HirExpr::Lit(HirLit::F64(6.0)))
    ));
}

#[test]
fn rejects_invalid_enum_native_layouts_and_members() {
    for (source, expected) in [
        (
            r#"enum Mixed { Number = 1, Text = "text" }
               function main(): void {}"#,
            "mixes numeric and string members",
        ),
        (
            r#"enum Text { First = "first", Second }
               function main(): void {}"#,
            "needs an initializer after a string member",
        ),
        (
            r#"enum Value { Present = 1 }
               function main(): void { console.log(Value.Missing); }"#,
            "has no member `Missing`",
        ),
        (
            r#"enum Text { Present = "present" }
               function main(): void { console.log(Text[0]); }"#,
            "does not support numeric reverse lookup",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn lowers_runtime_numeric_enum_reverse_lookup_as_optional_string() {
    let program = lower(
        r#"enum Status { Idle, Ready = 4, Alias = 4 }
           function read(index: number): string | undefined {
               return Status[index];
           }"#,
    );
    assert!(matches!(
        &program.functions[0].body[0],
        HirStmt::Return(Some(HirExpr::EnumReverseLookup(index, entries)))
            if matches!(index.as_ref(), HirExpr::Var(name) if name == "index")
                && entries == &vec![(0.0, "Idle".into()), (4.0, "Alias".into())]
    ));
}

#[test]
fn merges_compatible_enum_declarations_in_source_order() {
    let program = lower(
        r#"enum Status {}
           enum Status { First }
           enum Status { Second }
           enum Status { Third = 2 }
           function read(index: number): string | undefined {
               return Status[index];
           }"#,
    );
    assert!(matches!(
        &program.functions[0].body[0],
        HirStmt::Return(Some(HirExpr::EnumReverseLookup(_, entries)))
            if entries == &vec![(0.0, "Second".into()), (2.0, "Third".into())]
    ));

    for (source, expected) in [
        (
            r#"enum Value { First = 1 }
               enum Value { First = 2 }
               function main(): void {}"#,
            "duplicate member `First`",
        ),
        (
            r#"enum Value { First = 1 }
               enum Value { Text = "text" }
               function main(): void {}"#,
            "mixes numeric and string members",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn normalizes_same_layout_literal_unions_and_intersections() {
    let program = lower(
        r#"function text(value: "start" | "stop"): string { return value; }
           function numberValue(value: 1 | 2 | number): number { return value; }
           function flag(value: true | false): boolean { return value; }
           function intersection(value: string & "fixed"): string { return value; }
           function objectValue(value: { kind: "ready" | "waiting" }): string {
               return value.kind;
           }
           function values(input: ("a" | "b")[]): string[] { return input; }"#,
    );
    assert_eq!(program.functions[0].params[0].ty, HirType::Str);
    assert_eq!(program.functions[1].params[0].ty, HirType::F64);
    assert_eq!(program.functions[2].params[0].ty, HirType::Bool);
    assert_eq!(program.functions[3].params[0].ty, HirType::Str);
    assert!(matches!(
        &program.functions[4].params[0].ty,
        HirType::Object(fields) if fields == &vec![("kind".into(), HirType::Str)]
    ));
    assert_eq!(
        program.functions[5].params[0].ty,
        HirType::Array(Box::new(HirType::Str))
    );

    let module =
        thaw_parser::parse_typescript(r#"function mixed(value: string | number): void {}"#)
            .unwrap();
    let mixed = lower_module(&module).unwrap();
    assert_eq!(
        mixed.functions[0].params[0].ty,
        HirType::Union(vec![HirType::Str, HirType::F64])
    );
}

#[test]
fn lowers_heterogeneous_unions_to_tagged_injections() {
    let program = lower(
        r#"function identity(value: string | number): string | number { return value; }
           function kind(value: string | number): string { return typeof value; }
           function main(): void {
               const first: string | number = "text";
               const second: string | number = 2;
               kind(identity(first));
               kind(identity(second));
           }"#,
    );
    let union = HirType::Union(vec![HirType::Str, HirType::F64]);
    assert_eq!(program.functions[0].params[0].ty, union);
    assert_eq!(program.functions[0].ret, union);
    assert!(matches!(
        &program.functions[2].body[0],
        HirStmt::Let(_, ty, HirExpr::UnionInject(value, 0, members))
            if ty == &union
                && members == &vec![HirType::Str, HirType::F64]
                && matches!(value.as_ref(), HirExpr::Lit(HirLit::Str(_)))
    ));
    assert!(matches!(
        &program.functions[2].body[1],
        HirStmt::Let(_, ty, HirExpr::UnionInject(value, 1, members))
            if ty == &union
                && members == &vec![HirType::Str, HirType::F64]
                && matches!(value.as_ref(), HirExpr::Lit(HirLit::F64(2.0)))
    ));
}

#[test]
fn lowers_common_object_union_properties_and_discriminant_type_narrowing() {
    let program = lower(
        r#"type Result =
               { kind: number; value: number; shared: string } |
               { kind: string; value: string; shared: string };
           function describe(result: Result): string {
               console.log(result.shared);
               if (typeof result.kind === "number") {
                   return String(result.value + 1);
               }
               return result.value + "!";
           }"#,
    );
    let function = &program.functions[0];
    assert!(matches!(
        &function.body[0],
        HirStmt::Expr(HirExpr::Call(callee, arguments))
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "console.log")
            && matches!(arguments.as_slice(), [HirExpr::Call(lambda, _)]
                if matches!(lambda.as_ref(), HirExpr::Lambda(_, _, HirType::Str, _)))
    ));
    let HirStmt::If(_, then_body, _) = &function.body[1] else {
        panic!("expected the discriminant guard")
    };
    assert!(matches!(
        &then_body[0],
        HirStmt::Return(Some(HirExpr::Call(_, arguments)))
            if matches!(arguments.as_slice(), [HirExpr::BinOp(BinOp::Add, value, _) ]
                if matches!(value.as_ref(), HirExpr::PropAccess(object, _, field)
                    if field == "value" && matches!(object.as_ref(), HirExpr::UnionValue(_, 0, _))))
    ));
    assert!(matches!(
        &function.body[2],
        HirStmt::Return(Some(HirExpr::Call(_, arguments)))
            if matches!(arguments.as_slice(), [HirExpr::PropAccess(object, _, field), _]
                if field == "value" && matches!(object.as_ref(), HirExpr::UnionValue(_, 1, _)))
    ));
}

#[test]
fn rejects_properties_missing_from_an_object_union_member() {
    let module = thaw_parser::parse_typescript(
        r#"function read(value: { common: number } | { other: number }): number {
               return value.common;
           }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("a union member has no such field"),
        "{error}"
    );
}

#[test]
fn flattens_tagged_fields_read_from_object_unions() {
    let program = lower(
        r#"type Mixed =
               { kind: number; value: number | undefined } |
               { kind: string; value: string | null } |
               { kind: boolean; value: boolean | null | undefined };
           function show(value: Mixed): void { console.log(value.value); }"#,
    );
    let HirStmt::Expr(HirExpr::Call(_, arguments)) = &program.functions[0].body[0] else {
        panic!("expected console.log call")
    };
    let [HirExpr::Call(adapter, _)] = arguments.as_slice() else {
        panic!("expected union property adapter")
    };
    assert!(matches!(
        adapter.as_ref(),
        HirExpr::Lambda(
            _,
            _,
            HirType::Union(members),
            _
        ) if members == &vec![
            HirType::F64,
            HirType::Undefined,
            HirType::Str,
            HirType::Null,
            HirType::Bool,
        ]
    ));
}

#[test]
fn narrows_object_unions_by_literal_discriminants() {
    let program = lower(
        r#"type Result =
               { kind: "success"; value: number } |
               { kind: "failure"; value: string } |
               { kind: true; value: boolean };
           function describe(result: Result): string {
               if (result.kind === "success") return String(result.value + 1);
               if ("failure" === result.kind) return result.value + "!";
               return result.value ? "true" : "false";
           }"#,
    );
    let function = &program.functions[0];
    for (statement, expected_index) in function.body[..2].iter().zip([0, 1]) {
        let HirStmt::If(_, branch, _) = statement else {
            panic!("expected discriminant branch")
        };
        assert!(format!("{branch:?}")
            .contains(&format!("UnionValue(Var(\"result\"), {expected_index},")));
    }
    assert!(format!("{:?}", function.body[2]).contains("UnionValue(Var(\"result\"), 2,"));
}

#[test]
fn correlates_destructured_discriminants_with_sibling_unions() {
    let program = lower(
        r#"type Result =
               { kind: "success"; value: number; detail: number } |
               { kind: "failure"; value: string; detail: string };
           function describe(result: Result): string {
               const { kind: tag, value, detail } = result;
               if (tag === "success" && value > 0) {
                   return String(value + detail);
               }
               if ("failure" === tag) return value + detail;
               return "zero";
           }"#,
    );
    let body = format!("{:?}", program.functions[0].body);
    assert!(body.contains("UnionValue(Var(\"value\"), 0,"));
    assert!(body.contains("UnionValue(Var(\"detail\"), 0,"));
    assert!(body.contains("UnionValue(Var(\"value\"), 1,"));
    assert!(body.contains("UnionValue(Var(\"detail\"), 1,"));
}

#[test]
fn lowers_nested_object_and_tuple_destructuring_once() {
    let program = lower(
        r#"function source(): { x: number; label: string; nested: { flag: boolean }; extra: number } {
            return { x: 1, label: "ok", nested: { flag: true }, extra: 4 };
        }
        function main(): void {
            const { x: renamed, nested: { flag }, ...rest } = source();
            const [first, , pair, ...tail]: [number, string, { value: number }, number] =
                [1, "skip", { value: 3 }, 4];
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(matches!(
        &main.body[0],
        HirStmt::Let(name, HirType::Object(_), HirExpr::Call(_, _))
            if name.starts_with("__thaw_destructure_")
    ));
    assert!(main.body.iter().any(|statement| matches!(
        statement,
        HirStmt::Let(name, HirType::Bool, _) if name == "flag"
    )));
    assert!(main.body.iter().any(|statement| matches!(
        statement,
        HirStmt::Let(name, HirType::Array(element), _) if name == "tail" && element.as_ref() == &HirType::F64
    )));
}

#[test]
fn lowers_fixed_layout_destructuring_assignments() {
    let program = lower(
        r#"function source(): { x: number; label: string } {
            return { x: 1, label: "ok" };
        }
        function main(): void {
            let x = 0;
            let label = "";
            const returned = ({ x, label } = source());
            let first = 0;
            let tail = [0, 0];
            [first, ...tail] = [2, 3, 4];
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(matches!(
        &main.body[2],
        HirStmt::Let(_, HirType::Object(_), HirExpr::Call(_, _))
    ));
    assert!(matches!(
        main.body.last(),
        Some(HirStmt::Expr(HirExpr::Call(_, _)))
    ));
}

#[test]
fn lowers_for_of_destructuring_bindings_and_assignment_heads() {
    let program = lower(
        r#"function main(): void {
            const rows = [{ x: 1, label: "a" }];
            for (const { x, label } of rows) { console.log(x); }
            let assigned = 0;
            for ({ x: assigned } of rows) { console.log(assigned); }
            const pairs: [number, string][] = [[2, "b"]];
            for (const [value, text] of pairs) { console.log(text); }
        }"#,
    );
    let loops = program.functions[0]
        .body
        .iter()
        .filter_map(|statement| match statement {
            HirStmt::While(_, body) => Some(body),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(loops.len(), 3);
    assert!(loops.iter().all(|body| matches!(
        body.first(),
        Some(HirStmt::Let(name, _, _)) if name.starts_with("__thaw_for_of_item_")
    )));
}

#[test]
fn lowers_function_and_arrow_parameter_destructuring() {
    let program = lower(
        r#"function read(
            { x, nested: { flag }, ...rest }:
                { x: number; nested: { flag: boolean }; label: string },
            [first, ...tail]: [number, number, number]
        ): number { return x + first + tail[0]; }
        function main(): void {
            const pick = ({ value }: { value: number }): number => value;
            console.log(read(
                { x: 1, nested: { flag: true }, label: "ok" }, [2, 3, 4]
            ));
            console.log(pick({ value: 5 }));
        }"#,
    );
    let read = program
        .functions
        .iter()
        .find(|function| function.name == "read")
        .unwrap();
    assert!(read
        .params
        .iter()
        .all(|param| param.name.starts_with("__thaw_param_")));
    assert!(read.body.iter().any(|statement| matches!(
        statement,
        HirStmt::Let(name, HirType::Bool, _) if name == "flag"
    )));
}

#[test]
fn lowers_fixed_object_in_checks_with_operand_evaluation() {
    let program = lower(
        r#"function object(): { value: number } { return { value: 1 }; }
        function main(): void {
            console.log("value" in object());
            console.log("missing" in object());
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(main.body.iter().all(|statement| matches!(
        statement,
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::Call(_, _))
    )));
}

#[test]
fn rejects_dynamic_length_call_spread() {
    let module = thaw_parser::parse_typescript(
        r#"function emit(first: number): void {}
        function main(values: number[]): void { emit(...values); }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("statically known tuple length"), "{error}");
}

#[test]
fn lowers_fixed_object_for_in_to_key_array_loop() {
    let program = lower(
        r#"function main(): void {
            const object = { first: 1, second: 2 };
            for (const key in object) { console.log(key); }
        }"#,
    );
    let body = &program.functions[0].body;
    assert_eq!(body.len(), 5);
    assert!(matches!(
        &body[2],
        HirStmt::Let(_, HirType::Array(element), HirExpr::ArrayLit(keys))
            if element.as_ref() == &HirType::Str
                && keys == &[
                    HirExpr::Lit(HirLit::Str("first".into())),
                    HirExpr::Lit(HirLit::Str("second".into()))
                ]
    ));
    assert!(matches!(&body[4], HirStmt::While(_, _)));
}

#[test]
fn lowers_array_literal_index_and_length() {
    let program = lower(
        "function main(): void { const xs: number[] = [1, 2, 3]; console.log(xs[1]); console.log(xs.length); }",
    );
    let f = &program.functions[0];
    assert_eq!(
        f.body[0],
        HirStmt::Let(
            "xs".into(),
            HirType::Array(Box::new(HirType::F64)),
            HirExpr::ArrayLit(vec![
                HirExpr::Lit(HirLit::F64(1.0)),
                HirExpr::Lit(HirLit::F64(2.0)),
                HirExpr::Lit(HirLit::F64(3.0)),
            ]),
        )
    );
    assert_eq!(
        f.body[1],
        HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("console.log".into())),
            vec![HirExpr::TypedIndex(
                Box::new(HirExpr::Var("xs".into())),
                Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                HirType::F64,
            )],
        ))
    );
    assert_eq!(
        f.body[2],
        HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("console.log".into())),
            vec![HirExpr::ArrayLen(Box::new(HirExpr::Var("xs".into())))],
        ))
    );
}

#[test]
fn lowers_typed_array_spreads_in_source_order() {
    let program = lower(
        r#"function part(): number[] { return [2, 3]; }
           function main(): void {
               const tail: number[] = [4, 5];
               const values: number[] = [1, ...part(), ...tail, 6];
               console.log(values.length);
           }"#,
    );
    let HirStmt::Let(_, HirType::Array(element), HirExpr::ArrayConcat(parts, spread_element)) =
        &program.functions[1].body[1]
    else {
        panic!("expected typed array concat");
    };
    assert_eq!(element.as_ref(), &HirType::F64);
    assert_eq!(spread_element, &HirType::F64);
    assert_eq!(parts.len(), 4);
    assert!(matches!(&parts[0], HirExpr::ArrayLit(values) if values.len() == 1));
    assert!(matches!(&parts[1], HirExpr::Call(_, _)));
    assert!(matches!(&parts[2], HirExpr::Var(name) if name == "tail"));
    assert!(matches!(&parts[3], HirExpr::ArrayLit(values) if values.len() == 1));
}

#[test]
fn lowers_object_literal_field_access_and_mutation() {
    let program = lower(
        r#"function main(): void {
            const p: { x: number; y: number } = { x: 1, y: 2 };
            console.log(p.x);
            p.y = p.y + 1;
        }"#,
    );
    let f = &program.functions[0];
    let obj_ty = HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]);

    assert_eq!(
        f.body[0],
        HirStmt::Let(
            "p".into(),
            obj_ty.clone(),
            HirExpr::ObjectLit(vec![
                ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
            ]),
        )
    );
    assert_eq!(
        f.body[1],
        HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("console.log".into())),
            vec![HirExpr::PropAccess(
                Box::new(HirExpr::Var("p".into())),
                obj_ty.clone(),
                "x".into(),
            )],
        ))
    );
    assert_eq!(
        f.body[2],
        HirStmt::Expr(HirExpr::PropAssign(
            Box::new(HirExpr::Var("p".into())),
            obj_ty.clone(),
            "y".into(),
            Box::new(HirExpr::BinOp(
                BinOp::Add,
                Box::new(HirExpr::PropAccess(
                    Box::new(HirExpr::Var("p".into())),
                    obj_ty,
                    "y".into(),
                )),
                Box::new(HirExpr::Lit(HirLit::F64(1.0))),
            )),
        ))
    );
}

#[test]
fn lowers_object_literal_shorthand_properties() {
    let program = lower(
        r#"function main(): void {
            const x: number = 1;
            const label: string = "point";
            const point: { x: number; label: string } = { x, label };
            console.log(point.x);
        }"#,
    );

    assert_eq!(
        program.functions[0].body[2],
        HirStmt::Let(
            "point".into(),
            HirType::Object(vec![
                ("x".into(), HirType::F64),
                ("label".into(), HirType::Str),
            ]),
            HirExpr::ObjectLit(vec![
                ("x".into(), HirExpr::Var("x".into())),
                ("label".into(), HirExpr::Var("label".into())),
            ]),
        )
    );
}

#[test]
fn lowers_local_object_spread_and_later_property_overrides() {
    let program = lower(
        r#"function main(): void {
            const base: { x: number; label: string } = { x: 1, label: "base" };
            const point: { x: number; label: string } = { ...base, label: "point" };
            console.log(point.label);
        }"#,
    );
    let base_type = HirType::Object(vec![
        ("x".into(), HirType::F64),
        ("label".into(), HirType::Str),
    ]);

    assert_eq!(
        program.functions[0].body[1],
        HirStmt::Let(
            "point".into(),
            base_type.clone(),
            HirExpr::ObjectLit(vec![
                (
                    "x".into(),
                    HirExpr::PropAccess(
                        Box::new(HirExpr::Var("base".into())),
                        base_type,
                        "x".into(),
                    ),
                ),
                ("label".into(), HirExpr::Lit(HirLit::Str("point".into()))),
            ]),
        )
    );
}

#[test]
fn lowers_nested_object_literal_spread_without_reloading_fields() {
    let program = lower(
        r#"function main(): void {
            const point: { x: number; label: string } = {
                ...{ x: 1, label: "base" },
                label: "point"
            };
            console.log(point.x);
        }"#,
    );

    assert_eq!(
        program.functions[0].body[0],
        HirStmt::Let(
            "point".into(),
            HirType::Object(vec![
                ("x".into(), HirType::F64),
                ("label".into(), HirType::Str),
            ]),
            HirExpr::ObjectLit(vec![
                ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                ("label".into(), HirExpr::Lit(HirLit::Str("point".into()))),
            ]),
        )
    );
}

#[test]
fn evaluates_call_result_object_spread_once() {
    let program = lower(
        r#"function makeConfig(): { x: number; label: string } {
            return { x: 1, label: "base" };
        }
        function main(): void {
            const point: { x: number; label: string } = {
                ...makeConfig(),
                label: "point"
            };
            console.log(point.x);
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let HirStmt::Let(_, _, HirExpr::Call(lambda, arguments)) = &main.body[0] else {
        panic!("expected spread source to be bound through a lambda call");
    };
    assert!(matches!(
        arguments.as_slice(),
        [HirExpr::Call(callee, arguments)]
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "makeConfig")
                && arguments.is_empty()
    ));
    let HirExpr::Lambda(_, params, _, body) = lambda.as_ref() else {
        panic!("expected spread binding lambda");
    };
    assert_eq!(params.len(), 1);
    assert!(matches!(
        body.as_ref(),
        HirExpr::ObjectLit(fields)
            if matches!(&fields[0].1, HirExpr::PropAccess(object, _, field)
                if matches!(object.as_ref(), HirExpr::Var(name) if name == &params[0].name)
                    && field == "x")
                && fields[1] == ("label".into(), HirExpr::Lit(HirLit::Str("point".into())))
    ));
}

#[test]
fn reorders_object_literal_fields_to_match_the_declared_type() {
    // Written as {y, x} but the declared type says {x, y} -- lowering
    // should reorder so codegen only ever sees the declared order.
    let program = lower(
        r#"function main(): void {
            const p: { x: number; y: number } = { y: 2, x: 1 };
            console.log(p.x);
        }"#,
    );
    let f = &program.functions[0];
    assert_eq!(
        f.body[0],
        HirStmt::Let(
            "p".into(),
            HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]),
            HirExpr::ObjectLit(vec![
                ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
            ]),
        )
    );
}

#[test]
fn rejects_object_literal_with_wrong_field_type() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const p: { x: number } = { x: "not a number" };
        }"#,
    )
    .unwrap();
    assert!(lower_module(&module).is_err());
}

#[test]
fn coerces_object_literal_argument_to_the_parameter_shape() {
    let program = lower(
        r#"function dist(p: { x: number; y: number }): number {
            return p.x + p.y;
        }
        function main(): void {
            console.log(dist({ y: 2, x: 1 }));
        }"#,
    );
    let main = &program.functions[1];
    let HirStmt::Expr(HirExpr::Call(_, args)) = &main.body[0] else {
        panic!("expected a console.log call, got {:?}", main.body[0]);
    };
    let [console_arg] = args.as_slice() else {
        panic!("expected one argument to console.log");
    };
    let HirExpr::Call(_, dist_args) = console_arg else {
        panic!("expected a call to `dist`, got {console_arg:?}");
    };
    assert_eq!(
        dist_args,
        &vec![HirExpr::ObjectLit(vec![
            ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
            ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
        ])]
    );
}

#[test]
fn contextually_types_a_callback_inside_an_object_argument() {
    let program = lower(
        r#"function register(options: { handler: (request: { path: string }) => string }): void {}
        function main(): void {
            register({ handler: (request) => request.path });
        }"#,
    );
    let HirStmt::Expr(HirExpr::Call(_, arguments)) = &program.functions[1].body[0] else {
        panic!("expected register call");
    };
    assert!(matches!(
        arguments.as_slice(),
        [HirExpr::ObjectLit(fields)]
            if matches!(&fields[0].1, HirExpr::Lambda(_, params, HirType::Str, _)
                if params[0].ty == HirType::Object(vec![("path".into(), HirType::Str)]))
    ));
}

#[test]
fn contextually_types_a_callback_in_an_interface_method_call() {
    let module = thaw_parser::parse_typescript(
        r#"
        interface Source {
            on(event: 'data', callback: (chunk: string) => void): void;
        }
        declare function source(): Source;
        function main(): void {
            source().on('data', (chunk) => console.log(chunk));
            source().on('data', () => console.log('done'));
        }
        "#,
    )
    .unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn contextually_types_a_callback_passed_to_a_dynamic_function() {
    let module = thaw_parser::parse_typescript(
        r#"
        function invoke(subscribe: JsValue): void {
            subscribe((value) => console.log(value));
        }
        function main(): void {}
        "#,
    )
    .unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn dynamic_callback_preserves_explicit_native_signature() {
    let module = thaw_parser::parse_typescript(
        r#"
        function invoke(register: JsValue): void {
            register((value: number): number => value * 2);
        }
        function main(): void {}
        "#,
    )
    .unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn lowers_numeric_typed_array_constructors_through_the_dynamic_host() {
    let program = lower(
        r#"
        function main(): void {
            const bytes = new Uint8Array([1, 2, 255]);
            bytes[1] = 4;
            console.log(bytes.length, bytes[1]);
        }
        "#,
    );
    assert!(format!("{:?}", program.functions[0].body).contains("constructDynamicValue"));
    assert!(format!("{:?}", program.functions[0].body).contains("setDynamicPropertyJson"));
}

#[test]
fn lowers_abort_controller_through_the_dynamic_host() {
    let program = lower(
        r#"
        function main(): void {
            const controller = new AbortController();
            controller.abort("stop");
            console.log(controller.signal.aborted);
        }
        "#,
    );
    assert!(format!("{:?}", program.functions[0].body).contains("constructDynamicValue"));
}

/// `AbortSignal.timeout(ms)`/`AbortSignal.any(signals)` used to fail to
/// compile ("cannot infer the type of ... call to unknown function
/// AbortSignal.timeout") even though the QuickJS Fallback engine
/// already implements both correctly (`platform_globals/workers/
/// abort_timers.js`) -- the native compiler's dynamic-value receiver
/// inference (`infer_member_receiver_type`) recognized `Atomics`/
/// `crypto`/`process` as bridgeable global namespace objects but not
/// `AbortSignal`, and even once that's added, the identifier-lowering
/// match arm that turns a bare `AbortSignal` reference into a real
/// `getDynamicValue("AbortSignal")` call needs the same name added
/// too (found by rebuilding and hitting "unknown variable
/// `AbortSignal`" after only the first fix).
#[test]
fn lowers_abort_signal_static_methods_through_the_dynamic_host() {
    let program = lower(
        r#"
        function main(): void {
            const timeoutSignal = AbortSignal.timeout(50);
            console.log(timeoutSignal.aborted);
            const controller = new AbortController();
            const combined = AbortSignal.any([controller.signal]);
            console.log(combined.aborted);
        }
        "#,
    );
    let body = format!("{:?}", program.functions[0].body);
    assert!(body.contains("getDynamicValue"));
    assert!(body.contains("callDynamicMethod"));
}

#[test]
fn lowers_json_type_annotation() {
    let program = lower(
        r#"function wrap(args: Json): Json {
            return args;
        }
        function main(): void {}"#,
    );
    let f = &program.functions[0];
    assert_eq!(
        f.params,
        vec![HirParam {
            name: "args".into(),
            ty: HirType::Json
        }]
    );
    assert_eq!(f.ret, HirType::Json);
}

#[test]
fn lowers_number_conversion_through_native_object_stringification() {
    let program = lower("function main(): void { const x: number = Number({ value: 1 }); }");
    assert!(matches!(
        &program.functions[0].body[0],
        HirStmt::Let(_, HirType::F64, HirExpr::Call(callee, _))
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_string_to_number")
    ));
}

#[test]
fn rejects_indexed_assignment_into_a_non_array() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const data: string = "not an array";
            data[0] = 1;
        }"#,
    )
    .unwrap();
    assert!(lower_module(&module).is_err());
}

/// `JSON.parse` produces a `Json` value, and assigning a plain native value
/// into one of its indices used to be rejected outright (`coerce_to_declared`
/// had no general native-to-`Json` coercion, so the assigned `1` never
/// matched the declared `Json` element type) even though it's completely
/// ordinary JS. `coerce_to_declared`'s `Json` branch now boxes any
/// `json_convertible_native_type` value the same way `JSON.stringify`
/// already does for its own argument (`wrap_native_value_as_json`), so this
/// compiles like real JS would expect.
#[test]
fn assigns_a_native_value_into_a_json_indexed_slot() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const data = JSON.parse("[]");
            data[0] = 1;
        }"#,
    )
    .unwrap();
    assert!(lower_module(&module).is_ok());
}

/// A `Json`-typed receiver (from `JSON.parse`, same as the sibling test
/// above) calling a name this codebase also uses for a native array
/// builtin (`includes`) must still reach `lower_native_instance_
/// builtin`'s own clear "requires ... receiver, got Json" error --
/// not fall through to a generic dynamic-value dispatch path that
/// only actually supports a genuine `JsValue` receiver, which would
/// otherwise regress this into an opaque "call to undeclared function"
/// at codegen time instead. A real regression while fixing
/// `instanceof Date`/Date-method dispatch on a genuine `JsValue`
/// receiver (round 19's Date fix broadened this same gate to `JsValue
/// | Json` at first, catching a `Json` case it was never meant to
/// touch -- found via glob's own async `glob(...)` resolving to a
/// plain JSON array, see `[[project_npm_interop_gaps_20]]`).
/// A `Json`-typed value shaped `{"timestamp": N}` -- the shape a real
/// `Date` survives the QuickJS boundary as (`Date.prototype.toJSON` is
/// overridden globally, see `platform_globals/dates.js`) -- must be
/// recognized by `instanceof Date`, not hit the "not a known class"
/// error every other unrecognized class gets. Real trigger: js-yaml's
/// `load(input): unknown` (classified `Json`) with `YAML11_SCHEMA`,
/// yielding a real `Date`, see `[[project_npm_interop_gaps_19]]`.
#[test]
fn instanceof_date_recognizes_the_timestamp_shape_on_a_json_value() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): boolean {
            const data = JSON.parse('{"timestamp": 1579046400000}');
            return data instanceof Date;
        }"#,
    )
    .unwrap();
    assert!(lower_module(&module).is_ok());
}

/// The same `{"timestamp": N}`-shaped `Json` value calling a Date-
/// prototype-named method (`toISOString`) must be promoted to a real
/// native Date object and dispatch through the existing native Date
/// method machinery, rather than hitting `lower_native_instance_
/// builtin`'s own "receiver has type Json, expected Object(...)" error.
#[test]
fn date_method_call_promotes_a_json_timestamp_shape_to_native() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): string {
            const data = JSON.parse('{"timestamp": 1579046400000}');
            return data.toISOString();
        }"#,
    )
    .unwrap();
    assert!(lower_module(&module).is_ok());
}

/// `Number.isNaN`/`isFinite` on a `Json`-typed value must actually
/// inspect the decoded runtime value (via `JsonAsNumber`, backed by
/// `thaw_json_as_number`), not unconditionally return the compile-time
/// literal `false` the way every other non-`F64` type still does. A
/// bare `lower_module(...).is_ok()` check can't tell these two outcomes
/// apart -- lowering succeeds either way, only the *runtime answer*
/// differs -- so this inspects the compiled `main` body directly,
/// confirming it actually calls the same `__thaw_number_is_nan`
/// intrinsic the `F64` case already uses (via `HirExpr::JsonAsNumber`),
/// not a hardcoded `false` literal.
#[test]
fn number_is_nan_inspects_a_json_valued_operand_at_runtime() {
    let program = lower(
        r#"function main(): boolean {
            const value = JSON.parse('{"$__thaw_non_finite$": "NaN"}');
            return Number.isNaN(value);
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let body = format!("{:?}", main.body);
    assert!(body.contains("__thaw_number_is_nan"), "{body}");
    assert!(body.contains("JsonAsNumber"), "{body}");
    assert!(!body.contains("Bool(false)"), "{body}");
}

#[test]
fn a_json_valued_array_method_call_keeps_its_native_builtin_error() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): boolean {
            const data = JSON.parse("[1, 2, 3]");
            return data.includes(2);
        }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("requires"), "{error}");
    assert!(error.contains("Json"), "{error}");
    assert!(!error.contains("undeclared"), "{error}");
    assert!(!error.contains("unknown function"), "{error}");
}

#[test]
fn lowers_calls_through_function_typed_object_properties() {
    let program = lower(
        r#"interface Operations { apply: (value: number) => number; }
        function main(): void {
            const operations: Operations = {
                apply: (value: number): number => value + 1
            };
            console.log(operations.apply(41));
        }"#,
    );
    assert!(matches!(
        &program.functions[0].body[1],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::Call(callee, _)
                if matches!(callee.as_ref(), HirExpr::PropAccess(_, _, field) if field == "apply"))
    ));
}
