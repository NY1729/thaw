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
