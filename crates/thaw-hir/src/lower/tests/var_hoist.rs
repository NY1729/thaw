// `var` hoisting is applied only to vars used before (or outside) their declaration.

fn pre_read_names(source: &str) -> Vec<String> {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let statements: Vec<Stmt> = module
        .body
        .iter()
        .filter_map(|item| match item {
            swc_ecma_ast::ModuleItem::Stmt(stmt) => Some(stmt.clone()),
            _ => None,
        })
        .collect();
    pre_read_var_names(&statements)
}

#[test]
fn var_used_only_after_its_declaration_is_not_hoisted() {
    assert!(pre_read_names("var a = 1; console.log(a); { var b = 2; console.log(b); }").is_empty());
    let program = lower("function main(): void { var a = 1; console.log(a); }");
    let HirStmt::Let(_, ty, _) = &program.functions[0].body[0] else {
        panic!("expected a let")
    };
    assert_eq!(*ty, HirType::F64);
}

#[test]
fn var_read_before_or_outside_its_declaration_is_hoisted() {
    assert_eq!(pre_read_names("console.log(a); var a = 1;"), ["a"]);
    assert_eq!(
        pre_read_names("if (c) { var r = 1; } console.log(r);"),
        ["r"]
    );
    assert_eq!(pre_read_names("const g = () => k; var k = 4;"), ["k"]);
    assert!(pre_read_names("for (var i = 0; i < 2; i++) {} console.log(i);").is_empty());
    let program = lower("function main(): void { console.log(a); var a = 1; }");
    let HirStmt::Let(_, ty, _) = &program.functions[0].body[0] else {
        panic!("expected a let")
    };
    assert_eq!(*ty, HirType::Json);
}

#[test]
fn let_read_before_declaration_lowers_to_a_reference_error() {
    let program = lower("function main(): void { console.log(x); let x = 5; }");
    assert!(
        format!("{:?}", program.functions[0].body)
            .contains("Cannot access 'x' before initialization")
    );
}

#[test]
fn new_before_class_declaration_is_rewritten_to_a_reference_error() {
    let module = thaw_parser::parse_typescript(
        "try { console.log(new D().v); } catch (e) { console.log(1); }\nclass D { v = 1; }",
    )
    .unwrap();
    let normalized = normalize_module_for_lowering(&module).unwrap();
    assert!(format!("{normalized:?}").contains("@@thaw_tdz_throw"), "{normalized:?}");
}

#[test]
fn inner_scopes_that_redeclare_a_later_name_are_not_tdz_reads() {
    let program = lower(
        r#"
        function main(): void {
            let s = "";
            for (let i = 0; i < 3; i++) { s += i; }
            const fns: (() => number)[] = [];
            for (let k = 0; k < 2; k++) { fns.push(() => k); }
            let i = 0;
            let k = 1;
            console.log(s, i, k, fns.length);
        }
        "#,
    );
    let body = format!("{:?}", program.functions[0].body);
    assert!(!body.contains("before initialization"), "{body}");
    assert!(!body.contains("__thaw_tdz_"), "{body}");
}

#[test]
fn a_let_in_a_closure_does_not_make_a_later_var_hoisted() {
    assert!(pre_read_names("const f = (a: number) => a; function g(a: number) { return a; } var a = 1;").is_empty());
    assert!(pre_read_names("for (let i = 0; i < 2; i++) {} var i = 3;").is_empty());
}

#[test]
fn a_closure_parameter_named_like_a_later_const_is_not_a_tdz_read() {
    let program = lower(
        r#"
        async function main(): Promise<void> {
            const received = new Promise<Json>((resolve) => {
                setTimeout((value: Json) => { resolve(value); }, 1);
            });
            const value = await received;
            console.log(value);
        }
        "#,
    );
    let body = format!("{:?}", program.functions[0].body);
    assert!(!body.contains("before initialization"), "{body}");
}
