// `new.target` in top-level function declarations. Expected output from node 22.

#[test]
fn new_target_distinguishes_call_from_new() {
    let source = r#"
        function F(this: any): void {
            console.log(new.target === undefined, new.target === F);
        }
        function main(): void {
            F();
            new F();
        }
    "#;
    let output = compile_and_run(source, "new_target_distinguishes_call_from_new");
    assert_eq!(output, "true false\nfalse true\n");
}

#[test]
fn new_target_is_inherited_by_arrows_and_args_and_recursion_work() {
    let source = r#"
        function G(n: number): number {
            const kind = () => (new.target === undefined ? 1 : 2);
            if (n > 0) { return kind() * 10 + n + G(n - 1); }
            return kind();
        }
        function H(n: number): void {
            if (n > 0) { new H(n - 1); }
            console.log(n, new.target ? "new" : "call");
        }
        function main(): void {
            console.log(G(2));
            new G(2);
            H(1);
            new H(1);
        }
    "#;
    let output = compile_and_run(source, "new_target_arrow_args_recursion");
    assert_eq!(output, "24\n0 new\n1 call\n0 new\n1 new\n");
}
