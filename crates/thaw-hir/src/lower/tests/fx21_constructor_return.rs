#[test]
fn fx21_constructor_returning_another_class_instance_is_a_compile_error() {
    let module = thaw_parser::parse_typescript(
        "class B { q = 1; } class A { constructor() { return new B(); } } function main(): void { new A(); }",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("returns an instance of `B`"), "{error}");
}

#[test]
fn fx21_bare_constructor_return_lowers_without_error() {
    let module = thaw_parser::parse_typescript(
        "class A { x = 1; constructor(f: boolean) { if (f) return; this.x = 2; } } function main(): void { new A(true); }",
    )
    .unwrap();
    lower_module(&module).unwrap();
}
