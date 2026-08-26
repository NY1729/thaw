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
            const data = JSON.parse("[]");
            data[0] = 1;
        }"#,
    )
    .unwrap();
    assert!(lower_module(&module).is_err());
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

