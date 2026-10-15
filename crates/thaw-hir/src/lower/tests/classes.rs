#[test]
fn rejects_unreadable_or_non_numeric_instance_accessor_updates() {
    for (source, fragment) in [
        ("class Box { get value(): number { return 1; } } function main(): void { ++new Box().value; }", "readonly instance accessor"),
        ("class Box { get value(): string { return 'x'; } set value(next: string) {} } function main(): void { ++new Box().value; }", "non-number instance accessor"),
        ("class Box { set value(next: number) {} } function main(): void { ++new Box().value; }", "write-only instance accessor"),
        ("class Base { get value(): number { return 1; } set value(next: number) {} } class Derived extends Base { get value(): number { return 2; } } function main(): void { ++new Derived().value; }", "readonly instance accessor"),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(fragment), "{error}");
    }
}

#[test]
fn keeps_top_level_default_temporaries_private_when_exporting() {
    let module = thaw_parser::parse_typescript(
        "export const { value = 42 }: { value: number | undefined } = { value: undefined };",
    )
    .unwrap();
    let normalized = normalize_top_level_destructuring(&module).unwrap();
    let mut exported = Vec::new();
    let mut private_temporaries = Vec::new();
    for item in normalized.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                if let Decl::Var(declaration) = export.decl {
                    for declarator in declaration.decls {
                        if let Pat::Ident(binding) = declarator.name {
                            exported.push(binding.id.sym.to_string());
                        }
                    }
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration))) => {
                for declarator in declaration.decls {
                    if let Pat::Ident(binding) = declarator.name {
                        if binding.id.sym.starts_with("__thaw_top_") {
                            private_temporaries.push(binding.id.sym.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    assert_eq!(exported, ["value"]);
    assert!(private_temporaries
        .iter()
        .any(|name| name.starts_with("__thaw_top_default_")));
    assert!(private_temporaries
        .iter()
        .any(|name| name.starts_with("__thaw_top_destructure_")));
}

#[test]
fn keeps_computed_destructuring_key_temporaries_private_when_exporting() {
    let module = thaw_parser::parse_typescript(
        r#"const key: string = "selected";
           export const { [key]: selected, ...rest }: Record<string, number> =
               { selected: 1, kept: 2 };"#,
    )
    .unwrap();
    let normalized = normalize_top_level_destructuring(&module).unwrap();
    let mut exported = Vec::new();
    let mut private_keys = Vec::new();
    for item in normalized.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                if let Decl::Var(declaration) = export.decl {
                    for declarator in declaration.decls {
                        if let Pat::Ident(binding) = declarator.name {
                            exported.push(binding.id.sym.to_string());
                        }
                    }
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration))) => {
                for declarator in declaration.decls {
                    if let Pat::Ident(binding) = declarator.name {
                        if binding.id.sym.starts_with("__thaw_top_key_") {
                            private_keys.push(binding.id.sym.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    assert_eq!(exported, ["selected", "rest"]);
    assert_eq!(private_keys.len(), 1);
}

#[test]
fn lowers_static_computed_object_reads_and_targets() {
    let program = lower(
        r#"function main(): void {
            let point = { value: 1 };
            console.log(point["value"]);
            point["value"] = 2;
            point["value"] += 3;
            point["value"]++;
        }"#,
    );
    let body = &program.functions[0].body;
    assert!(matches!(
        &body[1],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::PropAccess(_, _, field) if field == "value")
    ));
    assert!(matches!(
        &body[2],
        HirStmt::Expr(HirExpr::PropAssign(_, _, field, _)) if field == "value"
    ));
    assert!(matches!(&body[3], HirStmt::Expr(HirExpr::Call(_, _))));
    assert!(matches!(&body[4], HirStmt::Expr(HirExpr::Call(_, _))));
}

#[test]
fn lowers_optional_chains_on_statically_non_null_values() {
    let program = lower(
        r#"function invoke(callback: (value: number) => number): number {
            return callback?.(2);
        }
        function main(): void {
            const box = { value: 1 };
            console.log(box?.value);
            console.log(box?.["value"]);
        }"#,
    );
    let invoke = program
        .functions
        .iter()
        .find(|function| function.name == "invoke")
        .unwrap();
    assert!(matches!(
        &invoke.body[0],
        HirStmt::Return(Some(HirExpr::Call(_, _)))
    ));
}

#[test]
fn lowers_static_and_tuple_call_argument_spreads() {
    let program = lower(
        r#"function emit(first: number, second: string, third: number): void {}
        function makeArgs(): [string, number] { return ["two", 3]; }
        function main(): void {
            emit(...[1, "two", 3]);
            emit(1, ...makeArgs());
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(matches!(
        &main.body[0],
        HirStmt::Expr(HirExpr::Call(lambda, _))
            if matches!(lambda.as_ref(), HirExpr::Lambda(_, _, HirType::Void, _))
    ));
    let HirStmt::Expr(HirExpr::Call(_, first_arguments)) = &main.body[1] else {
        panic!("expected bound leading argument");
    };
    assert!(matches!(
        first_arguments.as_slice(),
        [HirExpr::Lit(HirLit::F64(1.0))]
    ));
}

#[test]
fn desugars_classic_for_loop_into_let_and_while() {
    let program = lower(
        "function main(): void { for (let i = 0; i < 10; i = i + 1) { console.log(i); } }",
    );
    let f = &program.functions[0];
    assert_eq!(f.body.len(), 2);
    assert!(matches!(f.body[0], HirStmt::Let(ref n, HirType::F64, _) if n == "i"));
    let HirStmt::While(ref cond, ref body) = f.body[1] else {
        panic!("expected desugared while loop, got {:?}", f.body[1]);
    };
    assert_eq!(
        *cond,
        HirExpr::BinOp(
            BinOp::Lt,
            Box::new(HirExpr::Var("i".into())),
            Box::new(HirExpr::Lit(HirLit::F64(10.0))),
        )
    );
    // console.log(i) + the `i = i + 1` update appended to the body.
    assert_eq!(body.len(), 2);
    assert!(matches!(body[1], HirStmt::Expr(HirExpr::Assign(ref n, _)) if n == "i"));
}

#[test]
fn classic_for_continue_runs_the_update_but_nested_loop_continue_does_not() {
    let program = lower(
        r#"function main(): void {
            for (let i = 0; i < 3; i++) {
                while (i < 1) { continue; }
                if (i === 1) { continue; }
                console.log(i);
            }
        }"#,
    );
    let HirStmt::While(_, body) = &program.functions[0].body[1] else {
        panic!("expected desugared for loop");
    };
    let HirStmt::While(_, nested_body) = &body[0] else {
        panic!("expected nested while loop");
    };
    assert_eq!(nested_body, &[HirStmt::Continue]);
    let HirStmt::If(_, then_body, _) = &body[1] else {
        panic!("expected conditional continue");
    };
    assert!(matches!(
        then_body.as_slice(),
        [HirStmt::Expr(HirExpr::PostfixUpdate(_, BinOp::Add)), HirStmt::Continue]
    ));
    assert!(matches!(
        body.last(),
        Some(HirStmt::Expr(HirExpr::PostfixUpdate(_, BinOp::Add)))
    ));
}

#[test]
fn lowers_static_computed_object_literal_properties() {
    let program = lower(
        r#"function main(): void {
            const point: { x: number; label: string } = {
                ["x"]: 1,
                ["label"]: "point"
            };
            console.log(point.label);
        }"#,
    );

    assert_eq!(
        unstage_let(&program.functions[0].body[0]),
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
fn lowers_promise_constructor_then_and_catch_with_contextual_callbacks() {
    let program = lower(
        r#"async function main(): Promise<void> {
            const value: Promise<string> = new Promise<number>((resolve, reject) => {
                resolve(20);
            }).then(number => "ready");
            const recovered: Promise<number> = new Promise<number>((resolve, reject) => {
                reject("failure");
            }).catch(error => 42);
            console.log(await value);
            console.log(await recovered);
        }"#,
    );
    let HirStmt::Let(_, ty, HirExpr::PromiseThen(_, _, input, output, false, false)) =
        &program.functions[0].body[0]
    else {
        panic!("expected a typed Promise.then expression");
    };
    assert_eq!(ty, &HirType::Promise(Box::new(HirType::Str)));
    assert_eq!(input, &HirType::F64);
    assert_eq!(output, &HirType::Str);
    assert!(matches!(
        &program.functions[0].body[1],
        HirStmt::Let(
            _,
            HirType::Promise(_),
            HirExpr::PromiseThen(_, _, HirType::F64, HirType::F64, true, false)
        )
    ));
}

#[test]
fn lowers_promise_void_constructor_with_zero_argument_resolve() {
    let program = lower(
        r#"async function main(): Promise<void> {
            await new Promise<void>((resolve, reject) => {
                resolve();
            });
        }"#,
    );
    let HirStmt::Expr(HirExpr::Await(inner)) = &program.functions[0].body[0] else {
        panic!("expected awaited Promise<void>");
    };
    let HirExpr::PromiseNewMixed(executor, HirType::Void, resolver) = inner.as_ref() else {
        panic!("expected Promise<void> constructor");
    };
    assert!(matches!(resolver,
        HirType::CallableFunction(params, mask, None, ret)
            if mask.contains(0) && ret.as_ref() == &HirType::Void
                && matches!(params.as_slice(), [HirType::Optional(inner)]
                    if inner.as_ref() == &HirType::Promise(Box::new(HirType::Void)))
    ));
    let HirExpr::Lambda(_, params, HirType::Void, _) = executor.as_ref() else {
        panic!("expected Promise executor lambda");
    };
    assert!(matches!(
        &params[0].ty,
        HirType::CallableFunction(resolve_params, mask, None, ret)
            if resolve_params.len() == 1 && mask.contains(0)
                && ret.as_ref() == &HirType::Void
    ));
}

#[test]
fn promise_constructor_resolver_has_per_call_value_or_promise_type() {
    let program = lower(r#"
        async function main(): Promise<void> {
            const first: number = await new Promise<number>((resolve, reject) => {
                resolve(7);
                resolve(Promise.resolve(8));
            });
            const second: number = await new Promise<number>((resolve, reject) => {
                resolve(Promise.resolve(9));
                resolve(10);
            });
            const optional: number | undefined = await new Promise<number | undefined>((resolve) => {
                resolve(undefined);
                resolve(Promise.resolve(11));
            });
            console.log(first, second, optional);
        }
    "#);
    assert_eq!(program.functions.len(), 1);
    let source = r#"
        async function main(): Promise<void> {
            const result: number = await new Promise<number>((resolve) => {
                resolve("wrong");
            });
            console.log(result);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("Promise resolve value has type"), "{error}");
}

#[test]
fn infers_promise_constructor_type_and_reports_conflicting_resolves() {
    let program = lower(
        r#"async function main(): Promise<void> {
            const value: number = await new Promise((resolve, reject) => {
                resolve(42);
            });
            console.log(value);
        }"#,
    );
    assert!(matches!(
        &program.functions[0].body[0],
        HirStmt::Let(_, HirType::F64, HirExpr::AwaitPromise(_, HirType::F64))
    ));

    let local_program = lower(
        r#"async function main(): Promise<void> {
            const value: number = await new Promise((resolve, reject) => {
                const base = 20;
                const answer = base + 22;
                resolve(answer);
            });
            console.log(value);
        }"#,
    );
    assert!(matches!(
        &local_program.functions[0].body[0],
        HirStmt::Let(_, HirType::F64, HirExpr::AwaitPromise(_, HirType::F64))
    ));

    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const value = new Promise((resolve, reject) => {
                resolve(1);
                resolve("wrong");
            });
        }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("conflicting Promise resolve types"),
        "{error}"
    );
}

#[test]
fn specializes_forward_referenced_generic_classes_once_per_type_tuple() {
    let program = lower(
        r#"
        function read(value: Box<number>): number { return value.get(); }
        function main(): number {
            const first = new Box<number>(40);
            const duplicate = new Box<number>(2);
            const inferredDuplicate = new Box(1);
            const text = new Box<string>("ready");
            const pair = new Pair<string, number>(text.get(), first.get() + duplicate.get());
            return read(new Box<number>(pair.second + inferredDuplicate.get()));
        }
        class Pair<T, U> {
            constructor(public first: T, public second: U) {}
        }
        class Box<T> {
            constructor(public value: T) {}
            get(): T { return this.value; }
        }
        "#,
    );
    let number_box = specialized_generic_name("Box", &[HirType::F64]);
    let string_box = specialized_generic_name("Box", &[HirType::Str]);
    let pair = specialized_generic_name("Pair", &[HirType::Str, HirType::F64]);
    for class_name in [&number_box, &string_box, &pair] {
        assert_eq!(
            program
                .functions
                .iter()
                .filter(|function| {
                    function.name == class_constructor_symbol(class_name.as_ref())
                })
                .count(),
            1,
            "specialization {class_name} must be emitted exactly once"
        );
    }
    let number_getter = program
        .functions
        .iter()
        .find(|function| function.name == class_method_symbol(&number_box, "get"))
        .expect("number Box method specialization");
    assert_eq!(number_getter.ret, HirType::F64);
    let string_getter = program
        .functions
        .iter()
        .find(|function| function.name == class_method_symbol(&string_box, "get"))
        .expect("string Box method specialization");
    assert_eq!(string_getter.ret, HirType::Str);
}

#[test]
fn validates_native_generic_class_defaults_and_constraints() {
    for (source, expected) in [
        (
            "class Numeric<T extends number> { constructor(public value: T) {} } function main(): void { new Numeric<string>(\"wrong\"); }",
            "does not satisfy constraint F64",
        ),
        (
            "class Numeric<T extends number> { constructor(public value: T) {} } function main(): void { new Numeric(\"wrong\"); }",
            "does not satisfy constraint F64",
        ),
        (
            "class Numeric<T extends number> { static count: number = 0; } function main(): void { console.log(Numeric<string>.count); }",
            "does not satisfy constraint F64",
        ),
        (
            "class Invalid<T extends number = string> { constructor(public value: T) {} } function main(): void { new Invalid(\"wrong\"); }",
            "does not satisfy constraint F64",
        ),
        (
            "class Phantom<T> { constructor() {} } function main(): void { new Phantom(); }",
            "cannot infer generic class `Phantom` type parameter `T`",
        ),
        (
            "class Invalid<T = string, U> {} function main(): void {}",
            "required type parameter `U`",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn deduplicates_explicit_and_inferred_nested_generic_classes() {
    let program = lower(
        r#"
        class Box<T> { constructor(public value: T) {} }
        class Holder<T> { constructor(public value: T) {} }
        function main(): void {
            const boxed = new Box(42);
            const explicit = new Holder<Box<number>>(boxed);
            const inferred = new Holder(boxed);
            console.log(explicit.value.value + inferred.value.value);
        }
        "#,
    );
    assert_eq!(
        program
            .functions
            .iter()
            .filter(|function| {
                function.name.starts_with("__thaw_class_Holder__thaw_")
                    && function.name.ends_with("_constructor")
            })
            .count(),
        1
    );
}

#[test]
fn diagnoses_recursive_native_generic_class_layouts() {
    let module = thaw_parser::parse_typescript(
        r#"
        class Loop<T> { constructor(public next: Loop<T>) {} }
        function consume(value: Loop<number>): void {}
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("fixed-size native layout"), "{error}");
}

#[test]
fn lowers_one_shared_static_owner_for_generic_class_specializations() {
    let program = lower(
        r#"
        class Box<T> {
            static count: number = 0;
            constructor(public value: T) { Box.count += 1; }
        }
        function main(): void {
            new Box(1);
            new Box("two");
            console.log(Box.count);
        }
        "#,
    );
    let owner = generic_class_static_owner("Box");
    assert_eq!(
        program
            .globals
            .iter()
            .filter(|global| global.name == class_static_field_symbol(&owner, "count"))
            .count(),
        1
    );
    assert!(!program.globals.iter().any(|global| {
        global.name.contains("Box__thaw_f64_static_field_count")
            || global.name.contains("Box__thaw_str_static_field_count")
    }));
}

#[test]
fn rejects_generic_class_type_parameters_in_static_members() {
    let module = thaw_parser::parse_typescript(
        "class Invalid<T> { static value: T; } function main(): void {}",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("static members cannot reference class type parameter `T`"),
        "{error}"
    );
}

#[test]
fn specializes_explicit_generic_class_methods_once_per_type_tuple() {
    let program = lower(
        r#"
        class Box {
            convert<T>(value: T): T { return value; }
        }
        function main(): void {
            const box = new Box();
            console.log(box.convert<string>("first"));
            console.log(box.convert<string>("second"));
            console.log(box.convert("inferred"));
            console.log(box.convert<number>(42));
        }
        "#,
    );
    assert_eq!(
        program
            .functions
            .iter()
            .filter(|function| {
                function.name == class_method_symbol("Box", "convert__thaw_str")
            })
            .count(),
        1
    );
    assert!(program
        .functions
        .iter()
        .any(|function| { function.name == class_method_symbol("Box", "convert__thaw_f64") }));
}

#[test]
fn specializes_generic_class_returns_from_generic_methods_after_method_inference() {
    lower(
        r#"
        class Box<T> {
            value: T;
            constructor(value: T) { this.value = value; }
            map<U>(fn: (value: T) => U): Box<U> {
                return new Box<U>(fn(this.value));
            }
        }
        function main(): void {
            const value: string = new Box<number>(21)
                .map<string>((item: number): string => String(item * 2))
                .value;
            console.log(value);
        }
        "#,
    );
}

#[test]
fn infers_generic_class_methods_from_instance_member_types() {
    let program = lower(
        r#"
        class SourceBase {
            value: string = "field";
            get current(): string { return this.value; }
            read(): string { return this.current; }
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
        "#,
    );
    assert_eq!(
        program
            .functions
            .iter()
            .filter(|function| {
                function.name == class_method_symbol("SourceBase", "convert__thaw_str")
            })
            .count(),
        1
    );
}

#[test]
fn specializes_generic_methods_through_interface_typed_aliases() {
    lower(
        r#"
        interface Mapper {
            map<T, U>(value: T, fn: (value: T) => U): U;
        }
        class DirectMapper implements Mapper {
            map<T, U>(value: T, fn: (value: T) => U): U {
                return fn(value);
            }
        }
        function main(): void {
            const mapper: Mapper = new DirectMapper();
            console.log(mapper.map(
                21,
                value => String(value * 2),
            ));
        }
        "#,
    );
}

#[test]
fn validates_explicit_generic_class_method_arguments() {
    for (source, expected) in [
        (
            "class Box { numeric<T extends number>(value: T): T { return value; } } function main(): void { const box = new Box(); box.numeric<string>(\"bad\"); }",
            "does not satisfy constraint F64",
        ),
        (
            "class Box { pair<T, U>(left: T, right: U): T { return left; } } function main(): void { const box = new Box(); box.pair<number>(1, 2); }",
            "expects 2 type argument(s), got 1",
        ),
        (
            "class Box { numeric<T extends number>(value: T): T { return value; } } function main(): void { const box = new Box(); box.numeric(\"bad\"); }",
            "does not satisfy constraint F64",
        ),
        (
            "class Box { collect<T>(first: T, ...rest: T[]): T { return first; } } function main(): void { const box = new Box(); box.collect(1, \"bad\"); }",
            "conflicting call-site types",
        ),
        (
            "class Box { convert<T>(value: T): T { return value; } } function main(): void { const box = new Box(); const values: string[] = [\"bad\"]; box.convert(...values); }",
            "requires a statically sized tuple spread",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn validates_abstract_generic_class_method_implementations() {
    let valid = thaw_parser::parse_typescript(
        "abstract class Base { abstract convert<T>(value: T): T; } class Derived extends Base { convert<U>(value: U): U { return value; } } function main(): void { new Derived().convert(42); }",
    )
    .unwrap();
    lower_module(&valid).unwrap();

    for (source, expected) in [
        (
            "abstract class Base { abstract convert<T>(value: T): T; } class Derived extends Base {} function main(): void {}",
            "must implement abstract generic member `convert`",
        ),
        (
            "abstract class Base { abstract convert<T>(value: T): T; } class Derived extends Base { convert<T>(value: T): string { return \"bad\"; } } function main(): void {}",
            "abstract generic member `convert` from `Base` with an incompatible signature",
        ),
        (
            "abstract class Base { abstract convert<T extends number>(value: T): T; } class Derived extends Base { convert<T extends string>(value: T): T { return value; } } function main(): void {}",
            "abstract generic member `convert` from `Base` with an incompatible signature",
        ),
        (
            "abstract class Base { abstract collect<T>(...value: T[]): T; } class Derived extends Base { collect<T>(value: T[]): T { return value[0]; } } function main(): void {}",
            "abstract generic member `collect` from `Base` with an incompatible signature",
        ),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn lowers_native_class_construction_and_this_field_initialization() {
    let program = lower(
        r#"class Counter {
            value: number = 1;
            label: string;
            constructor(value: number, label: string) {
                this.value = value;
                this.label = label;
            }
        }
        function main(): number {
            const counter = new Counter(42, "ready");
            return counter.value;
        }"#,
    );
    let constructor = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Counter_constructor")
        .expect("native class constructor");
    assert_eq!(constructor.params.len(), 2);
    assert!(matches!(
        &constructor.body[0],
        HirStmt::Let(name, HirType::Object(fields), HirExpr::ClassAlloc(_))
            if name == "__thaw_this"
                && fields.iter().any(|(name, ty)| name == "value" && ty == &HirType::F64)
                && fields.iter().any(|(name, ty)| name == "label" && ty == &HirType::Str)
    ));
    let initializer = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Counter_initialize")
        .expect("native class initializer");
    assert_eq!(initializer.params[0].name, "__thaw_this");
    assert!(matches!(
        constructor.body.last(),
        Some(HirStmt::Return(Some(HirExpr::Call(callee, args))))
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_class_Counter_initialize")
                && matches!(args.first(), Some(HirExpr::Var(name)) if name == "__thaw_this")
    ));
    assert_eq!(
        initializer
            .body
            .iter()
            .filter(|statement| matches!(statement, HirStmt::Expr(HirExpr::PropAssign(..))))
            .count(),
        3
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(matches!(
        &main.body[0],
        HirStmt::Let(_, HirType::Object(_), HirExpr::Call(callee, _))
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_class_Counter_constructor")
    ));
}

#[test]
fn lowers_native_class_instance_methods_with_explicit_receiver() {
    let program = lower(
        r#"class Counter {
            value: number;
            constructor(value: number) { this.value = value; }
            add(delta: number): number {
                this.value += delta;
                return this.value;
            }
        }
        function main(): number {
            const counter = new Counter(40);
            return counter.add(2);
        }"#,
    );
    let method = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Counter_method_add")
        .expect("native class method");
    assert_eq!(method.params[0].name, "__thaw_this");
    assert!(matches!(method.params[0].ty, HirType::Object(_)));
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    // The receiver (`counter`) and the argument are staged into call-argument temporaries and
    // the receiver is checked for class identity before it is passed explicitly.
    let lowered = format!("{:?}", main.body);
    assert!(lowered.contains(
        "Call(Var(\"__thaw_class_Counter_method_add\"), [Call(Var(\"__thaw_assert_class_identity\"), [Var(\"__thaw_class_call_arg_"
    ), "{lowered}");
    assert!(lowered.contains("Lit(Str(\"Counter\"))") && lowered.contains("Lit(F64(2.0))"), "{lowered}");
    assert!(lowered.contains("[Var(\"counter\")]"), "{lowered}");
}

#[test]
fn lowers_native_class_static_methods_without_a_receiver() {
    let program = lower(
        r#"class MathBox {
            static add(left: number, right: number): number { return left + right; }
        }
        function main(): number { return MathBox.add(40, 2); }"#,
    );
    let method = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_MathBox_static_add")
        .expect("native static method");
    assert_eq!(method.params.len(), 2);
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(format!("{:?}", main.body).contains(
        "Call(Var(\"__thaw_class_MathBox_static_add\"), [Lit(F64(40.0)), Lit(F64(2.0))])"
    ));
}

#[test]
fn lowers_native_class_instance_and_static_getters() {
    let program = lower(
        r#"class Box {
            value: number;
            constructor(value: number) { this.value = value; }
            get doubled(): number { return this.value * 2; }
            static get version(): string { return "v1"; }
        }
        function main(): number {
            const box = new Box(21);
            console.log(Box.version);
            return box.doubled;
        }"#,
    );
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == "__thaw_class_Box_instance_getter_doubled"));
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == "__thaw_class_Box_static_getter_version"));
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let body = format!("{:?}", main.body);
    assert!(body.contains("__thaw_class_Box_static_getter_version"));
    assert!(body.contains("__thaw_class_Box_instance_getter_doubled"));
}

#[test]
fn lowers_native_class_setters_and_preserves_assignment_values() {
    let program = lower(
        r#"let version: number = 0;
        class Box {
            stored: number;
            constructor(value: number) { this.stored = value; }
            set value(next: number) { this.stored = next; }
            static set current(next: number) { version = next; }
        }
        function main(): number {
            const box = new Box(1);
            const assigned = (box.value = 40);
            const selected = (Box.current = 2);
            return assigned + selected + version;
        }"#,
    );
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == "__thaw_class_Box_instance_setter_value"));
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == "__thaw_class_Box_static_setter_current"));
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let body = format!("{:?}", main.body);
    assert!(body.contains("__thaw_class_Box_instance_setter_value"));
    assert!(body.contains("__thaw_class_Box_static_setter_current"));
    let setter = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Box_instance_setter_value")
        .unwrap();
    assert_eq!(setter.ret, HirType::Void);
    assert!(!setter.body.iter().any(|statement| matches!(statement,
        HirStmt::Return(Some(_))
    )));
}

#[test]
fn lowers_constructor_parameter_properties_as_instance_fields() {
    let program = lower(
        r#"class Point {
            constructor(public x: number, readonly label: string) {}
            sum(y: number): number { return this.x + y; }
        }
        function main(): number {
            const point = new Point(40, "ready");
            console.log(point.label);
            return point.sum(2);
        }"#,
    );
    let constructor = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Point_constructor")
        .unwrap();
    let HirType::Object(fields) = &constructor.ret else {
        panic!("class layout")
    };
    assert!(fields
        .iter()
        .any(|(name, ty)| name == "x" && ty == &HirType::F64));
    assert!(fields
        .iter()
        .any(|(name, ty)| name == "label" && ty == &HirType::Str));
    let initializer = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Point_initialize")
        .unwrap();
    assert_eq!(
        initializer
            .body
            .iter()
            .filter(|statement| matches!(statement, HirStmt::Expr(HirExpr::PropAssign(..))))
            .count(),
        2
    );
}

#[test]
fn builds_forward_class_inheritance_layouts_in_base_to_derived_order() {
    let program = lower(
        r#"class Derived extends Base {
            label: string;
            read(): number { return this.value; }
        }
        class Base { value: number; }
        function main(): number {
            const value = new Derived();
            return value.read();
        }"#,
    );
    let constructor = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Derived_constructor")
        .unwrap();
    let HirType::Object(fields) = &constructor.ret else {
        panic!("derived layout")
    };
    assert_eq!(
        fields,
        &vec![
            ("__thaw_class_identity_\u{1e}Derived\u{1f}Base".into(), HirType::Bool),
            ("value".into(), HirType::F64),
            ("label".into(), HirType::Str),
        ]
    );
}

#[test]
fn rejects_class_inheritance_cycles_and_field_collisions() {
    let cycle = thaw_parser::parse_typescript(
        "class First extends Second {} class Second extends First {}",
    )
    .unwrap();
    assert!(lower_module(&cycle)
        .unwrap_err()
        .contains("class inheritance cycle"));

    let collision = thaw_parser::parse_typescript(
        "class Base { value: number; } class Derived extends Base { value: number; }",
    )
    .unwrap();
    assert!(lower_module(&collision)
        .unwrap_err()
        .contains("collides with an inherited or local field"));
}

#[test]
fn lowers_super_to_the_base_initializer_on_the_same_instance() {
    let program = lower(
        r#"class Derived extends Base {
            label: string = "ready";
            constructor(value: number) { super(value); }
            answer(): number { return this.value; }
        }
        class Base { constructor(public value: number) {} }
        function main(): number { return new Derived(42).answer(); }"#,
    );
    let initializer = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Derived_initialize")
        .unwrap();
    // The `super(...)` arguments are evaluated first (staged as `__thaw_native_arg_N`), then the
    // base initializer is invoked on the same `__thaw_this` instance.
    let super_call = format!("{:?}", initializer.body[0]);
    assert!(matches!(&initializer.body[0], HirStmt::Expr(HirExpr::Call(_, _))), "{super_call}");
    assert!(
        super_call.contains(
            "Call(Var(\"__thaw_class_Base_initialize\"), [Var(\"__thaw_this\"), Var(\"__thaw_native_arg_0\")])"
        ),
        "{super_call}"
    );
    assert!(matches!(
        &initializer.body[1],
        HirStmt::Expr(HirExpr::PropAssign(_, _, field, _)) if field == "label"
    ));
}

#[test]
fn lowers_super_method_calls_to_the_direct_base_implementation() {
    let program = lower(
        r#"class Base {
            constructor(public value: number) {}
            answer(delta: number): number { return this.value + delta; }
        }
        class Derived extends Base {
            constructor(value: number) { super(value); }
            answer(delta: number): number { return super.answer(delta) + 1; }
        }
        function main(): number { return new Derived(40).answer(1); }"#,
    );
    let method = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Derived_method_answer")
        .unwrap();
    // The argument is staged (`__thaw_native_arg_N`) before the direct base call on `__thaw_this`.
    let lowered = format!("{:?}", method.body);
    assert!(
        lowered.contains(
            "Call(Var(\"__thaw_class_Base_method_answer\"), [Var(\"__thaw_this\"), Var(\"__thaw_native_arg_0\")])"
        ),
        "{lowered}"
    );
    assert!(lowered.contains("[Var(\"delta\")]"), "{lowered}");
}

#[test]
fn lowers_super_getter_and_setter_access_to_base_accessors() {
    let program = lower(
        r#"class Base {
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
        function main(): number {
            const value = new Derived(1);
            value.value = 20;
            return value.value;
        }"#,
    );
    let getter = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Derived_instance_getter_value")
        .unwrap();
    assert!(format!("{:?}", getter.body).contains("__thaw_class_Base_instance_getter_value"));
    let setter = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Derived_instance_setter_value")
        .unwrap();
    assert!(format!("{:?}", setter.body).contains("__thaw_class_Base_instance_setter_value"));
}

#[test]
fn inherits_static_members_and_prefers_static_overrides() {
    let program = lower(
        r#"let stored: number = 0;
        class Base {
            static add(left: number, right: number): number { return left + right; }
            static get current(): number { return stored; }
            static set current(next: number) { stored = next; }
        }
        class Derived extends Base {
            static add(left: number, right: number): number { return left + right + 1; }
        }
        function main(): number {
            Derived.current = 40;
            return Derived.current + Derived.add(1, 1);
        }"#,
    );
    for symbol in [
        "__thaw_class_Derived_static_getter_current",
        "__thaw_class_Derived_static_setter_current",
    ] {
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == symbol));
    }
    assert_eq!(
        program
            .functions
            .iter()
            .filter(|function| function.name == "__thaw_class_Derived_static_add")
            .count(),
        1
    );
}

#[test]
fn lowers_static_super_methods_getters_and_setters() {
    let program = lower(
        r#"let stored: number = 0;
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
        function main(): number {
            Derived.current = 40;
            return Derived.current + Derived.add(0);
        }"#,
    );
    for symbol in [
        "__thaw_class_Derived_static_add",
        "__thaw_class_Derived_static_getter_current",
        "__thaw_class_Derived_static_setter_current",
    ] {
        let function = program
            .functions
            .iter()
            .find(|function| function.name == symbol)
            .unwrap();
        assert!(format!("{:?}", function.body).contains("__thaw_class_Base_static"));
    }
}

#[test]
fn forwards_implicit_derived_constructor_arguments_through_multiple_levels() {
    let program = lower(
        r#"class Leaf extends Middle {}
        class Middle extends Base {}
        class Base {
            constructor(public value: number, public label: string) {}
            answer(): number { return this.value; }
        }
        function main(): number {
            const value = new Leaf(42, "ready");
            console.log(value.label);
            return value.answer();
        }"#,
    );
    for class_name in ["Middle", "Leaf"] {
        let constructor = program
            .functions
            .iter()
            .find(|function| function.name == format!("__thaw_class_{class_name}_constructor"))
            .unwrap();
        assert_eq!(constructor.params.len(), 2);
    }
    let leaf_initializer = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_Leaf_initialize")
        .unwrap();
    assert!(format!("{:?}", leaf_initializer.body).contains("__thaw_class_Middle_initialize"));
}

#[test]
fn validates_native_class_implements_against_inherited_layout() {
    let program = lower(
        r#"interface NamedValue<N, V> { name: N; value: V; }
        class Named {
            constructor(public name: string) {}
        }
        class Value extends Named implements NamedValue<string, number> {
            constructor(name: string, public value: number) { super(name); }
        }
        function main(): number { return new Value("answer", 42).value; }"#,
    );
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == "__thaw_class_Value_constructor"));
}

#[test]
fn validates_native_class_implements_methods_without_adding_layout_fields() {
    let program = lower(
        r#"interface Named { name(): string; }
        class User implements Named {
            private value: string;
            constructor(value: string) { this.value = value; }
            name(): string { return this.value; }
        }
        function main(): void {
            const user: Named = new User("Alice");
            console.log(user.name());
        }"#,
    );
    let HirType::Object(fields) = &program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_class_User_constructor")
        .expect("User constructor")
        .ret
    else {
        panic!("class constructor should return an object");
    };
    assert!(!fields.iter().any(|(name, _)| name == "name"));
}

#[test]
fn rejects_native_class_missing_an_implemented_field() {
    let module = thaw_parser::parse_typescript(
        r#"interface Required { value: number; label: string; }
        class Incomplete implements Required {
            constructor(public value: number) {}
        }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("missing field `label` required by `Required`"),
        "{error}"
    );
}

#[test]
fn rejects_native_class_implemented_field_type_mismatch() {
    let module = thaw_parser::parse_typescript(
        r#"type Required = { value: number };
        class Mismatch implements Required {
            constructor(public value: string) {}
        }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("field `value` has type Str, but `Required` requires F64"),
        "{error}"
    );
}

#[test]
fn lowers_initialized_native_static_fields_as_globals() {
    let program = lower(
        r#"class Counter {
            static base: number = 40;
            static value: number = Counter.base + 2;
            static readonly label: string = "ready";
            static next(): number { Counter.value += 1; return Counter.value; }
        }
        function main(): number { console.log(Counter.label); return Counter.next(); }"#,
    );
    for field in ["base", "value", "label"] {
        assert!(program
            .globals
            .iter()
            .any(|global| { global.name == class_static_field_symbol("Counter", field) }));
    }
    assert!(program.initializers.iter().any(|step| matches!(
        step,
        HirInitStep::StoreGlobal(name, _)
            if name == &class_static_field_symbol("Counter", "value")
    )));
}

#[test]
fn lowers_generic_static_this_initializers_on_the_shared_owner() {
    let program = lower(
        r#"class Box<T> {
            static label: string = "ready";
            static identity<U>(value: U): U { return value; }
            static initialized: string = this.identity<string>(this.label);
            constructor(public value: T) {}
        }
        function main(): void { new Box(1); console.log(Box.initialized); }"#,
    );
    let owner = generic_class_static_owner("Box");
    for field in ["label", "initialized"] {
        let symbol = class_static_field_symbol(&owner, field);
        assert!(
            program.globals.iter().any(|global| global.name == symbol),
            "missing shared static global {symbol}"
        );
    }
}

#[test]
fn rejects_assignment_to_readonly_native_static_field() {
    let module = thaw_parser::parse_typescript(
        r#"class Constants { static readonly answer: number = 42; }
        function main(): void { Constants.answer = 0; }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("cannot assign to constant"), "{error}");

    let through_this = thaw_parser::parse_typescript(
        r#"class Constants {
            static readonly answer: number = 42;
            static invalid(): number { return this.answer++; }
        }"#,
    )
    .unwrap();
    let error = lower_module(&through_this).unwrap_err();
    assert!(error.contains("cannot update constant"), "{error}");
}

#[test]
fn inherits_native_static_fields_without_copying_storage() {
    let program = lower(
        r#"class Base {
            static value: number = 40;
            static readonly label: string = "shared";
        }
        class Middle extends Base {}
        class Leaf extends Middle {}
        function main(): number { Leaf.value = 42; return Base.value; }"#,
    );
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == class_getter_symbol("Leaf", "value", true)));
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == class_setter_symbol("Leaf", "value", true)));
    assert!(!program
        .globals
        .iter()
        .any(|global| { global.name == class_static_field_symbol("Leaf", "value") }));
    assert!(!program
        .functions
        .iter()
        .any(|function| function.name == class_setter_symbol("Leaf", "label", true)));
}

#[test]
fn rejects_assignment_to_inherited_readonly_native_static_field() {
    let module = thaw_parser::parse_typescript(
        r#"class Base { static readonly label: string = "fixed"; }
        class Derived extends Base {}
        function main(): void { Derived.label = "changed"; }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("cannot assign to readonly static member `Derived.label`"),
        "{error}"
    );
}

#[test]
fn lowers_native_static_blocks_in_class_body_order() {
    let program = lower(
        r#"let trace: string = "";
        class Counter {
            static value: number = 1;
            static { Counter.value += 40; trace += "A"; }
            static result: number = Counter.value + 1;
            static { trace += "B"; }
        }
        function main(): number { console.log(trace); return Counter.result; }"#,
    );
    let value = class_static_field_symbol("Counter", "value");
    let result = class_static_field_symbol("Counter", "result");
    let value_index = program
        .initializers
        .iter()
        .position(|step| matches!(step, HirInitStep::StoreGlobal(name, _) if name == &value))
        .unwrap();
    let result_index = program
        .initializers
        .iter()
        .position(|step| matches!(step, HirInitStep::StoreGlobal(name, _) if name == &result))
        .unwrap();
    assert!(result_index > value_index + 1);
}

#[test]
fn lowers_string_literal_computed_native_class_members() {
    let program = lower(
        r#"class Box {
            ["value"]: number;
            static ["count"]: number = 40;
            constructor(value: number) { this["value"] = value; }
            ["add"](delta: number): number { return this["value"] + delta; }
            get ["current"](): number { return this["value"]; }
            set ["current"](value: number) { this["value"] = value; }
            static ["next"](): number { return ++Box["count"]; }
        }
        function main(): number {
            const value = new Box(40);
            value["current"] = value["add"](2);
            return value["current"] + Box["next"]();
        }"#,
    );
    for symbol in [
        class_method_symbol("Box", "add"),
        class_getter_symbol("Box", "current", false),
        class_setter_symbol("Box", "current", false),
        class_static_method_symbol("Box", "next"),
    ] {
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == symbol));
    }
}

#[test]
fn folds_static_computed_native_class_member_names() {
    let program = lower(
        r#"const prefix = "val";
        const field = `${prefix}ue` as const;
        const method = ("re" + "ad") as string;
        class Box {
            [field]: number = 42;
            [method](): number { return this[field]; }
        }
        function main(): number { return new Box()[method](); }"#,
    );
    assert!(program
        .functions
        .iter()
        .any(|function| function.name == class_method_symbol("Box", "read")));

    let module = thaw_parser::parse_typescript(
        r#"let key: string = "value";
        class Box { [key]: number = 42; }
        function main(): void {}"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("computed members require a string-literal name"),
        "{error}"
    );
}

#[test]
fn lowers_tuple_spreads_for_native_class_constructors_and_methods() {
    let program = lower(
        r#"class Calculator {
            constructor(public offset: number) {}
            sum(left: number, right: number): number {
                return this.offset + left + right;
            }
            static sum(left: number, right: number): number { return left + right; }
        }
        function main(): number {
            const constructorArgs: [number] = [1];
            const args: [number, number] = [20, 21];
            const calculator = new Calculator(...constructorArgs);
            return calculator.sum(...args) + Calculator.sum(...args);
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let body = format!("{:?}", main.body);
    assert!(body.contains("__thaw_class_Calculator_constructor"));
    assert!(body.contains("__thaw_class_Calculator_method_sum"));
    assert!(body.contains("__thaw_class_Calculator_static_sum"));
}

#[test]
fn lowers_typed_tuple_spreads_for_native_method_bind() {
    let program = lower(
        r#"class Binder {
            join(left: string, right: string): string { return left + right; }
            static join(left: string, right: string): string { return left + right; }
        }
        function main(): void {
            const binder = new Binder();
            const first: [string] = ["left"];
            const both: [string, string] = ["left", "right"];
            const instanceBound = binder.join.bind(binder, ...first);
            const staticBound = Binder.join.bind(binder, ...both);
            console.log(instanceBound("right"));
            console.log(staticBound());
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let body = format!("{:?}", main.body);
    assert!(body.contains("__thaw_class_Binder_method_join"));
    assert!(body.contains("__thaw_class_Binder_static_join"));
    assert_eq!(body.matches("__thaw_native_spread_").count(), 6);

    let module = thaw_parser::parse_typescript(
        r#"class Binder { join(value: string): string { return value; } }
        function main(): void {
            const binder = new Binder();
            const values: string[] = ["value"];
            binder.join.bind(binder, ...values);
        }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("spread source must have statically known tuple length"),
        "{error}"
    );
}

#[test]
fn extracts_this_independent_native_methods_as_function_values() {
    let program = lower(
        r#"class Operations {
            pass(value: string): string { return value; }
            static double(value: number): number { return value * 2; }
        }
        function main(): void {
            const operations = new Operations();
            const pass = operations.pass;
            const double = Operations.double;
            console.log(pass("extracted"));
            console.log(double(21));
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let body = format!("{:?}", main.body);
    assert!(body.contains("__thaw_class_Operations_method_pass"));
    assert!(body.contains("__thaw_class_Operations_static_double"));
    assert!(body.contains("Lambda"));
    assert!(body.contains("FunctionRef"));
}

#[test]
fn invokes_saved_this_dependent_methods_with_call_and_apply() {
    let program = lower(
        r#"class Box {
            constructor(public value: string) {}
            read(suffix: string): string { return this.value + suffix; }
            maybe(useThis: boolean): string {
                if (useThis) return this.value;
                return this === undefined ? "undefined-this" : "wrong-this";
            }
        }
        class StaticBox {
            static value: string = "static";
            static read(suffix: string): string { return this.value + suffix; }
        }
        function main(): void {
            const first = new Box("first");
            const second = new Box("second");
            const read = first.read;
            const alias = read;
            const args: [string] = ["?"];
            const boundArgs: [string] = ["!"];
            const bound = alias.bind(second, ...boundArgs);
            const staticRead = StaticBox.read;
            const staticBound = staticRead.bind(first, ...boundArgs);
            const maybe = first.maybe;
            console.log(read.call(second, "!"));
            console.log(alias.apply(first, args));
            console.log(bound());
            console.log(staticRead.call(first, "!"));
            console.log(staticRead.apply(first, args));
            console.log(staticBound());
            console.log(maybe(false));
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(format!("{:?}", main.body).contains("__thaw_class_Box_method_read"));
    assert!(program.functions.iter().any(|function| {
        function.name == unbound_class_method_symbol(&class_method_symbol("Box", "maybe"))
    }));
}

#[test]
fn lowers_tuple_spreads_for_super_constructors_and_methods() {
    let program = lower(
        r#"class Base {
            constructor(public left: number, public right: number) {}
            sum(left: number, right: number): number { return left + right; }
            static sum(left: number, right: number): number { return left + right; }
        }
        class Derived extends Base {
            constructor(args: [number, number]) { super(...args); }
            sumPair(args: [number, number]): number { return super.sum(...args); }
            static sumPair(args: [number, number]): number { return super.sum(...args); }
        }
        function main(): number {
            const args: [number, number] = [20, 22];
            return new Derived(args).sumPair(args) + Derived.sumPair(args);
        }"#,
    );
    for symbol in [
        class_initializer_symbol("Derived"),
        class_method_symbol("Derived", "sumPair"),
        class_static_method_symbol("Derived", "sumPair"),
    ] {
        let function = program
            .functions
            .iter()
            .find(|function| function.name == symbol)
            .unwrap();
        assert!(format!("{:?}", function.body).contains("__thaw_native_spread_"));
    }
}

#[test]
fn lowers_static_block_super_fields_methods_and_accessors() {
    let program = lower(
        r#"class Base {
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
        function main(): number { return Derived.after; }"#,
    );
    // Skip the class-token `loadScript` init statements (one per
    // value-referenced class) and find the static-block statement itself.
    let block = program
        .initializers
        .iter()
        .find(|step| {
            matches!(step, HirInitStep::Statement(_))
                && format!("{step:?}").contains("__thaw_class_Base_static_setter_current")
        })
        .unwrap();
    let debug = format!("{block:?}");
    assert!(debug.contains("__thaw_class_Base_static_setter_current"));
    assert!(debug.contains("__thaw_class_Base_static_field_value"));
}

#[test]
fn lowers_native_instanceof_across_the_inheritance_chain() {
    let program = lower(
        r#"class Base {}
        class Middle extends Base {}
        class Leaf extends Middle {}
        class Other {}
        function main(): boolean {
            const value = new Leaf();
            return value instanceof Leaf && value instanceof Middle
                && value instanceof Base && !(value instanceof Other);
        }"#,
    );
    let HirType::Object(fields) = &program
        .functions
        .iter()
        .find(|function| function.name == class_constructor_symbol("Leaf"))
        .unwrap()
        .ret
    else {
        panic!("leaf constructor must return an object")
    };
    assert_eq!(fields[0].0, "__thaw_class_identity_\u{1e}Leaf\u{1f}Middle\u{1f}Base");
}

#[test]
fn dollar_sign_in_class_name_does_not_split_nominal_ancestry() {
    let program = lower(
        r#"class Base$Name {}
           class Leaf extends Base$Name {}
           function main(): boolean {
               const value = new Leaf();
               return value instanceof Base$Name;
           }"#,
    );
    let class = program
        .functions
        .iter()
        .find(|function| function.name == class_constructor_symbol("Leaf"))
        .unwrap();
    let HirType::Object(fields) = &class.ret else {
        panic!("leaf constructor must return an object")
    };
    assert_eq!(fields[0].0, "__thaw_class_identity_\u{1e}Leaf\u{1f}Base$Name");
}

#[test]
fn lowers_native_class_default_parameter_wrappers() {
    let program = lower(
        r#"class Box {
            constructor(public value: number = 40, public label: string = String(value)) {}
            add(delta: number = this.value): number { return this.value + delta; }
            static sum(left: number = 20, right: number = left + 22): number {
                return left + right;
            }
        }
        function main(): number {
            const value = new Box();
            console.log(value.label);
            return value.add() + Box.sum();
        }"#,
    );
    for symbol in [
        default_arity_symbol(&class_constructor_symbol("Box"), 0),
        default_arity_symbol(&class_method_symbol("Box", "add"), 1),
        default_arity_symbol(&class_static_method_symbol("Box", "sum"), 0),
    ] {
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == symbol));
    }
}

#[test]
fn rejects_constructing_an_abstract_native_class() {
    let module = thaw_parser::parse_typescript(
        r#"abstract class Shape { abstract area(): number; }
        function main(): void { const value = new Shape(); }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("cannot construct abstract class `Shape`"),
        "{error}"
    );
}

#[test]
fn rejects_missing_or_incompatible_abstract_implementations() {
    let missing = thaw_parser::parse_typescript(
        r#"abstract class Shape { abstract area(value: number): number; }
        class Missing extends Shape {}"#,
    )
    .unwrap();
    let error = lower_module(&missing).unwrap_err();
    assert!(
        error.contains("must implement abstract member `area`"),
        "{error}"
    );

    let incompatible = thaw_parser::parse_typescript(
        r#"abstract class Shape { abstract area(value: number): number; }
        class Wrong extends Shape { area(value: string): number { return 0; } }"#,
    )
    .unwrap();
    let error = lower_module(&incompatible).unwrap_err();
    assert!(error.contains("incompatible signature"), "{error}");

    let missing_field = thaw_parser::parse_typescript(
        r#"abstract class Named { abstract name: string; }
        class MissingField extends Named {}"#,
    )
    .unwrap();
    let error = lower_module(&missing_field).unwrap_err();
    assert!(
        error.contains("must implement abstract field `name`"),
        "{error}"
    );

    let wrong_field = thaw_parser::parse_typescript(
        r#"abstract class Named { abstract name: string; }
        class WrongField extends Named { name: number = 1; }"#,
    )
    .unwrap();
    let error = lower_module(&wrong_field).unwrap_err();
    assert!(
        error.contains("implements abstract field `name`"),
        "{error}"
    );
}

#[test]
fn rejects_uninitialized_non_optional_native_static_fields() {
    let module =
        thaw_parser::parse_typescript("class Invalid { static value: number; }").unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("needs an optional or undefined-capable type"),
        "{error}"
    );
}

#[test]
fn generic_class_method_type_parameter_shadows_class_parameter() {
    let program = lower(
        r#"
        class Box<T> {
            constructor(public value: T) {}
            get(): T { return this.value; }
            identity<T>(value: T): T { return value; }
        }
        function main(): void {
            const box = new Box<string>("ready");
            const text: string = box.get();
            const number: number = box.identity<number>(42);
            console.log(text, number);
        }
        "#,
    );
    let string_box = specialized_generic_name("Box", &[HirType::Str]);
    let getter = program.functions.iter().find(|function|
        function.name == class_method_symbol(&string_box, "get")).unwrap();
    assert_eq!(getter.ret, HirType::Str);
    let identity = program.functions.iter().find(|function|
        function.name.starts_with(&class_method_symbol(&string_box, "identity__thaw_f64")))
        .expect("method-local T specializes independently of class T");
    assert_eq!(identity.ret, HirType::F64);
}

#[test]
fn generic_type_substitution_respects_nested_type_binders_and_conditional_arms() {
    #[derive(Default)]
    struct References(Vec<String>);
    impl Visit for References {
        fn visit_ts_type_ref(&mut self, reference: &swc_ecma_ast::TsTypeRef) {
            if reference.type_params.is_none() {
                if let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name {
                    self.0.push(name.sym.to_string());
                }
            }
            reference.visit_children_with(self);
        }
    }
    let mut module = thaw_parser::parse_typescript(
        r#"
        type FunctionShape = [T, <T>(value: T) => T, T];
        type MappedShape = { [T in keyof T]: T };
        type InferShape = T extends infer T ? T : T;
        type NestedInfer = T extends (string extends infer T ? T : never) ? T : T;
        type AliasShape<T> = T;
        type SignatureShape = { <T>(value: T): T; new <T>(value: T): T; method<T>(value: T): T };
        interface InterfaceShape<T> { value: T; }
        "#,
    ).unwrap();
    let replacement = TsType::TsKeywordType(swc_ecma_ast::TsKeywordType {
        span: swc_common::DUMMY_SP,
        kind: TsKeywordTypeKind::TsStringKeyword,
    });
    let substitutions = HashMap::from([("T".to_string(), Box::new(replacement))]);
    for item in &mut module.body {
        item.visit_mut_with(&mut GenericClassTypeSubstituter::new(&substitutions));
    }
    let mut references = HashMap::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(alias))) => {
                let mut names = References::default();
                alias.visit_with(&mut names);
                references.insert(alias.id.sym.to_string(), names.0);
                if alias.id.sym == "NestedInfer" {
                    let TsType::TsConditionalType(outer) = alias.type_ann.as_ref() else {
                        panic!("outer conditional");
                    };
                    assert!(matches!(outer.true_type.as_ref(), TsType::TsKeywordType(_)),
                        "nested infer must not shadow the outer true arm");
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(interface))) => {
                let mut names = References::default();
                interface.visit_with(&mut names);
                references.insert(interface.id.sym.to_string(), names.0);
            }
            _ => {}
        }
    }
    assert_eq!(references["FunctionShape"], ["T", "T"]);
    assert_eq!(references["MappedShape"], ["T"]);
    assert_eq!(references["InferShape"], ["T"]);
    assert_eq!(references["NestedInfer"], ["T"]);
    assert_eq!(references["AliasShape"], ["T"]);
    assert_eq!(references["SignatureShape"], ["T", "T", "T", "T", "T", "T"]);
    assert_eq!(references["InterfaceShape"], ["T"]);
}

#[test]
fn generic_static_and_instance_methods_keep_separate_templates_in_both_orders() {
    for methods in [
        "static convert<T>(value: T): string { return 'static'; } convert<T>(value: T): T { return value; }",
        "convert<T>(value: T): T { return value; } static convert<T>(value: T): string { return 'static'; }",
    ] {
        let source = format!(r#"
            class Box {{ {methods} }}
            function main(): void {{
                const box = new Box();
                const text: string = Box.convert<number>(4);
                const value: number = box.convert<number>(4);
                const inferredText: string = Box.convert(5);
                const inferredValue: number = box.convert(5);
                console.log(text, value, inferredText, inferredValue);
            }}
        "#);
        let program = lower(&source);
        for symbol in [
            class_static_method_symbol("Box", "convert__thaw_f64"),
            class_method_symbol("Box", "convert__thaw_f64"),
        ] {
            assert_eq!(program.functions.iter().filter(|function| function.name == symbol).count(), 1);
        }
    }
}

#[test]
fn specializes_generic_static_and_instance_method_instantiations() {
    let program = lower(r#"
        class Source {
            static convert<T>(value: T): string { return "static"; }
            convert<T>(value: T): T { return value; }
        }
        function main(): void {
            const source = new Source();
            const staticMethod = Source.convert<number>;
            const instanceMethod = source.convert<number>;
            const text: string = staticMethod(4);
            const number: number = instanceMethod(4);
            console.log(text, number);
        }
    "#);
    for symbol in [
        class_static_method_symbol("Source", "convert__thaw_f64"),
        class_method_symbol("Source", "convert__thaw_f64"),
    ] {
        assert_eq!(program.functions.iter().filter(|function| function.name == symbol).count(), 1);
    }
}

#[test]
fn specializes_inherited_generic_methods_by_this_and_super_receiver_kind() {
    let program = lower(r#"
        class Base {
            static convert<T>(value: T): string { return "static"; }
            convert<T>(value: T): T { return value; }
        }
        class Derived extends Base {
            static fromThis(): string { return this.convert<number>(1); }
            fromThis(): number { return this.convert<number>(2); }
            static fromSuper(): string { return super.convert<number>(3); }
            fromSuper(): number { return super.convert<number>(4); }
        }
        function main(): void {
            const derived = new Derived();
            console.log(Derived.fromThis(), derived.fromThis(), Derived.fromSuper(), derived.fromSuper());
        }
    "#);
    for symbol in [
        class_static_method_symbol("Base", "convert__thaw_f64"),
        class_method_symbol("Base", "convert__thaw_f64"),
    ] {
        assert_eq!(program.functions.iter().filter(|function| function.name == symbol).count(), 1);
    }
}

#[test]
fn generic_method_collection_respects_class_name_shadowing() {
    let program = lower(r#"
        class Source {
            static convert<T>(value: T): string { return "static"; }
            convert<T>(value: T): T { return value; }
        }
        class Other { convert<T>(value: T): T { return value; } }
        function shadow(Source: Other): number { return Source.convert<number>(7); }
        function main(): void {
            const source = new Source();
            console.log(Source.convert<number>(1), source.convert<number>(2), shadow(new Other()));
        }
    "#);
    for symbol in [
        class_static_method_symbol("Source", "convert__thaw_f64"),
        class_method_symbol("Source", "convert__thaw_f64"),
        class_method_symbol("Other", "convert__thaw_f64"),
    ] {
        assert_eq!(program.functions.iter().filter(|function| function.name == symbol).count(), 1);
    }
}

#[test]
fn shadowed_class_name_keeps_instance_generic_method_call_apply_bind_and_saved_this() {
    let program = lower(r#"
        class Box {
            value: number = 1;
            static value: number = 10;
            static convert<T>(item: T): T { this.value; return item; }
            convert<T>(item: T): T { this.value; return item; }
        }
        function shadow(Box: Box): number {
            const args: [number] = [2];
            const direct: number = Box.convert<number>.call(Box, 2);
            const applied: number = Box.convert<number>.apply(Box, args);
            const bound = Box.convert<number>.bind(Box, 3);
            const saved = Box.convert<number>;
            const stored: number = saved.call(Box, 4);
            return direct + applied + bound() + stored;
        }
        function main(): void {
            console.log(Box.convert<number>(1), shadow(new Box()));
        }
    "#);
    let shadow = program.functions.iter().find(|function| function.name == "shadow").unwrap();
    let body = format!("{:?}", shadow.body);
    assert!(body.contains(&class_method_symbol("Box", "convert__thaw_f64")), "{body}");
    assert!(!body.contains(&class_static_method_symbol("Box", "convert__thaw_f64")), "{body}");
    for symbol in [
        class_method_symbol("Box", "convert__thaw_f64"),
        class_static_method_symbol("Box", "convert__thaw_f64"),
    ] {
        assert_eq!(program.functions.iter().filter(|function| function.name == symbol).count(), 1);
    }
}

#[test]
fn infers_generic_method_types_from_expanded_tuple_spread_positions() {
    let program = lower(r#"
        class Mapper {
            pair<T, U, V>(first: T, second: U, third: V): [T, U, V] {
                return [first, second, third];
            }
            collect<T>(first: T, ...rest: T[]): T { return first; }
            keep<T, U = string>(value: T): T { return value; }
        }
        function main(): void {
            const mapper = new Mapper();
            const prefix: [number, string] = [1, "two"];
            const numbers: [number, number] = [3, 4];
            const singleton: [number] = [5];
            const result: [number, string, boolean] = mapper.pair(...prefix, true);
            const collected: number = mapper.collect(...numbers);
            const kept: number = mapper.keep(...singleton);
            console.log(result, collected, kept);
        }
    "#);
    for symbol in [
        class_method_symbol("Mapper", &specialized_generic_name("pair", &[HirType::F64, HirType::Str, HirType::Bool])),
        class_method_symbol("Mapper", &specialized_generic_name("collect", &[HirType::F64])),
        class_method_symbol("Mapper", &specialized_generic_name("keep", &[HirType::F64, HirType::Str])),
    ] {
        assert_eq!(program.functions.iter().filter(|function| function.name == symbol).count(), 1);
    }
}

#[test]
fn rejects_tuple_spread_that_exceeds_generic_method_parameters() {
    let module = thaw_parser::parse_typescript(r#"
        class Mapper { pair<T, U>(first: T, second: U): T { return first; } }
        function main(): void {
            const mapper = new Mapper();
            const values: [number, string, boolean] = [1, "two", true];
            mapper.pair(...values);
        }
    "#).unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("too many arguments"), "{error}");
}

#[test]
fn tuple_spread_shifts_contextual_generic_callback_position() {
    let program = lower(r#"
        class Mapper {
            map<T, U, V>(first: T, second: U, callback: (value: T) => V): V {
                return callback(first);
            }
        }
        function main(): void {
            const mapper = new Mapper();
            const prefix: [number, string] = [7, "seven"];
            const result: string = mapper.map(...prefix, value => String(value * 2));
            console.log(result);
        }
    "#);
    let symbol = class_method_symbol(
        "Mapper",
        &specialized_generic_name("map", &[HirType::F64, HirType::Str, HirType::Str]),
    );
    assert_eq!(program.functions.iter().filter(|function| function.name == symbol).count(), 1);
}

#[test]
fn normalized_constructor_static_writes_keep_original_statement_order() {
    let source = "let state: number = 0; function Box() { this.value = 0; } \
        let before: number | undefined = Box.count; state = 4; \
        Box.count = state + 0; Box.count = state + 1; ++Box.count;";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let normalized = normalize_constructor_functions(&module).unwrap();
    let class = normalized.body.iter().find_map(|item| match item {
        ModuleItem::Stmt(Stmt::Decl(Decl::Class(class))) if class.ident.sym == *"Box" => Some(class),
        _ => None,
    }).unwrap();
    let generated: Vec<_> = class.class.body.iter().filter_map(|member| match member {
        ClassMember::ClassProp(field) if field.is_static => Some(field),
        _ => None,
    }).collect();
    assert_eq!(generated.len(), 1);
    assert!(generated[0].value.is_none());
    assert!(generated[0].is_optional);
    let assignments = normalized.body.iter().filter(|item| matches!(item,
        ModuleItem::Stmt(Stmt::Expr(swc_ecma_ast::ExprStmt { expr, .. }))
            if matches!(expr.as_ref(), Expr::Assign(_)))).count();
    // state=4 and both writes to Box.count remain separate statements.
    assert_eq!(assignments, 3);
    let _ = lower_module(&module).unwrap();
}

#[test]
fn normalized_constructor_static_optional_update_lowers_before_first_write() {
    let source = "function Box() { this.value = 0; } \
        let previous: number = Box.count++; Box.count = 2; \
        let updated: number = ++Box.count; Box.count += 1;";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let _ = lower_module(&module).unwrap();
}

#[test]
fn normalized_constructor_repeated_mixed_value_kinds_share_one_static_field() {
    let module = thaw_parser::parse_typescript(
        "function Box() { this.value = 0; } Box.mixed = 1; Box.mixed = 'later';"
    ).unwrap();
    let normalized = normalize_constructor_functions(&module).unwrap();
    let class = normalized.body.iter().find_map(|item| match item {
        ModuleItem::Stmt(Stmt::Decl(Decl::Class(class))) if class.ident.sym == *"Box" => Some(class),
        _ => None,
    }).unwrap();
    assert_eq!(class.class.body.iter().filter(|member| matches!(member,
        ClassMember::ClassProp(field) if field.is_static)).count(), 1);
    let _ = lower_module(&module).unwrap();
}
#[test]
fn native_string_union_add_does_not_construct_bigint_host_calls() {
    let program = lower(
        r#"function optional(present: boolean): string | undefined {
            return present ? "S" : undefined;
        }
        function choose(present: boolean): string | number {
            return present ? "S" : 2;
        }
        function nativeOptional(): any { return optional(true) + 2; }
        function nativeUnion(): any { return choose(true) + 2; }
        function nativeJson(value: any): any { return value + 2; }"#,
    );
    for name in ["nativeOptional", "nativeUnion", "nativeJson"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        let body = format!("{:?}", function.body);
        assert!(!body.contains("callDynamicValue"), "{name}: {body}");
        assert!(!body.contains("getDynamicValue"), "{name}: {body}");
    }
}

// Unrun regression: a public spelling cannot collide with a normalized # slot.
#[test]
fn preserves_private_slot_identity_with_public_name_collision() {
    let module = thaw_parser::parse_typescript(r#"
        export class Vault {
            #value: number = 1;
            __thaw_private_Vault_value: number = 2;
            read(): number { return this.#value; }
        }
    "#).unwrap();
    PRIVATE_CLASS_SLOTS.with(|slots| slots.borrow_mut().clear());
    let normalized = normalize_private_class_members(&module);
    let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) = &normalized.body[0] else {
        panic!("exported class was not preserved");
    };
    let Decl::Class(class) = &export.decl else { panic!("expected class"); };
    let names = class.class.body.iter().filter_map(|member| match member {
        ClassMember::ClassProp(property) => Some(class_property_name(&property.key).unwrap()),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(names, ["__thaw_private_Vault_value_", "__thaw_private_Vault_value"]);
    let ClassMember::Method(method) = &class.class.body[2] else { panic!("expected method"); };
    let Stmt::Return(returned) = &method.function.body.as_ref().unwrap().stmts[0] else {
        panic!("expected return");
    };
    let Expr::Member(read) = returned.arg.as_deref().unwrap() else { panic!("expected read"); };
    assert!(matches!(&read.prop, MemberProp::Ident(key) if key.sym == *"__thaw_private_Vault_value_"));
}

// Unrun regression: nested private declarations and reads remain in their scope.
#[test]
fn keeps_nested_private_class_scope_out_of_outer_normalization() {
    let module = thaw_parser::parse_typescript(r#"
        class Outer {
            #value: number = 1;
            #base: any;
            #outerValue: number = 5;
            inner() { return class Inner extends this.#base {
                #value: number = 3;
                #base: number = 4;
                read(): number { return this.#value; }
                outer(other: Outer): number { return other.#outerValue; }
            }; }
        }
    "#).unwrap();
    PRIVATE_CLASS_SLOTS.with(|slots| slots.borrow_mut().clear());
    let normalized = normalize_private_class_members(&module);
    let ModuleItem::Stmt(Stmt::Decl(Decl::Class(outer))) = &normalized.body[0] else {
        panic!("expected outer class");
    };
    let ClassMember::Method(method) = &outer.class.body[3] else { panic!("expected method"); };
    let Stmt::Return(returned) = &method.function.body.as_ref().unwrap().stmts[0] else {
        panic!("expected return");
    };
    let Expr::Class(inner) = returned.arg.as_deref().unwrap() else { panic!("expected inner class"); };
    assert!(matches!(&inner.class.body[0], ClassMember::PrivateProp(property) if property.key.name == *"value"));
    let ClassMember::Method(read) = &inner.class.body[2] else { panic!("expected read method"); };
    let Stmt::Return(returned) = &read.function.body.as_ref().unwrap().stmts[0] else { panic!("expected return"); };
    let Expr::Member(read) = returned.arg.as_deref().unwrap() else { panic!("expected private read"); };
    assert!(matches!(&read.prop, MemberProp::PrivateName(name) if name.name == *"value"));
    let Expr::Member(heritage) = inner.class.super_class.as_deref().unwrap() else { panic!("expected heritage read"); };
    assert!(matches!(&heritage.prop, MemberProp::Ident(name) if name.sym == *"__thaw_private_Outer_base"));
    let ClassMember::Method(method) = &inner.class.body[3] else { panic!("expected outer read"); };
    let Stmt::Return(returned) = &method.function.body.as_ref().unwrap().stmts[0] else { panic!("expected return"); };
    let Expr::Member(read) = returned.arg.as_deref().unwrap() else { panic!("expected outer private read"); };
    assert!(matches!(&read.prop, MemberProp::Ident(name) if name.sym == *"__thaw_private_Outer_outerValue"));
}

#[test]
fn computed_class_key_folding_respects_initialized_declaration_order() {
    for (source, expected) in [
        ("const KEY = 'm'; class C { [KEY](): number { return 1; } }", Some("m")),
        ("class C { [KEY](): number { return 1; } } const KEY = 'm';", None),
        ("const A = 'm', KEY = A; class C { [KEY](): number { return 1; } }", Some("m")),
        ("const KEY = A, A = 'm'; class C { [KEY](): number { return 1; } }", None),
        ("export class C { [KEY](): number { return 1; } } export const KEY = 'm';", None),
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let normalized = normalize_static_computed_class_members(&module);
        let class = normalized.body.iter().find_map(|item| match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(class))) => Some(class),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
                Decl::Class(class) => Some(class),
                _ => None,
            },
            _ => None,
        }).unwrap();
        let method = class.class.body.iter().find_map(|member| match member {
            ClassMember::Method(method) => Some(method),
            _ => None,
        }).unwrap();
        match expected {
            Some(expected) => {
                let PropName::Str(name) = &method.key else { panic!("key not folded: {source}"); };
                assert_eq!(name.value.to_string_lossy(), expected);
            }
            None => {
                let ClassMember::StaticBlock(block) = &class.class.body[0] else { panic!("missing runtime TDZ guard: {source}"); };
                assert!(matches!(&block.body.stmts[0], Stmt::Throw(_)), "{source}");
            },
        }
    }
}

#[test]
fn class_key_order_guard_preserves_object_keys_in_method_bodies() {
    struct Keys(Vec<String>);
    impl Visit for Keys {
        fn visit_prop_name(&mut self, property: &PropName) {
            if let PropName::Str(key) = property {
                self.0.push(key.value.to_string_lossy().into_owned());
            }
            property.visit_children_with(self);
        }
    }
    let module = thaw_parser::parse_typescript(
        "const K = 'x'; class C { m(): { x: number } { return { [K]: 1 }; } }",
    ).unwrap();
    let normalized = normalize_static_computed_class_members(&module);
    let mut keys = Keys(Vec::new());
    normalized.visit_with(&mut keys);
    assert!(keys.0.iter().any(|key| key == "x"));
}

#[test]
fn forward_class_key_throws_during_initialization_even_when_class_is_unused() {
    let program = lower(
        "const ReferenceError = 17; class C { [KEY](): number { return 1; } } const KEY = 'm'; function main(): void {}",
    );
    let thrown = program.initializers.iter().position(|step|
        matches!(step, HirInitStep::Statement(HirStmt::Throw(_)))
    ).unwrap();
    let key_initialized = program.initializers.iter().position(|step|
        matches!(step, HirInitStep::StoreGlobal(name, _) if name == "KEY")
    ).unwrap();
    assert!(thrown < key_initialized);
    // A throw now publishes a staged thrown value (`__thaw_thrown_value_N`), so the message lives
    // in the initializer steps that stage it just before the throw.
    let message = format!("{:?}", &program.initializers[..=thrown]);
    assert!(message.contains("ReferenceError"), "{message}");
    assert!(message.contains("before initialization"), "{message}");
}

#[test]
fn initialized_class_key_does_not_gain_a_synthetic_tdz_throw() {
    let program = lower(
        "const KEY = 'm'; class C { [KEY](): number { return 1; } } function main(): number { return new C().m(); }",
    );
    assert!(!program.initializers.iter().any(|step|
        matches!(step, HirInitStep::Statement(HirStmt::Throw(_)))
    ));
    assert!(program.functions.iter().any(|function| function.name == class_method_symbol("C", "m")));
}
