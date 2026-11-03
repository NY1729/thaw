#[test]
fn live_object_literal_with_methods_carries_its_members_for_thaw_js() {
    // A method-bearing literal passed to a `Json` parameter is wrapped in the member marker, the
    // builder block kept as its first operand (see `lower/objects.rs`).
    let program = lower(
        r#"declare function sink(opts: any): void;

        function main(): void {
            sink({ init(a: number) { return a; }, n: 1 });
        }"#,
    );
    let dump = format!("{:?}", program.functions);
    assert!(
        dump.contains("Var(\"__thaw_dynamic_object_literal\")"),
        "method-bearing literal lost its member marker: {dump}"
    );
}
