#[test]
fn r8f_contract_caught_error_keeps_identity_through_callback() {
    let source = r#"
        function main(): void {
            const err = new Error("x");
            const invoke: any = (cb: any): any => cb();
            try {
                invoke(() => { throw err; });
            } catch (e: any) {
                console.log(e === err, e.message, e.name);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "r8f_contract_caught_error_keeps_identity_through_callback"),
        "true x Error\n"
    );
}

#[test]
fn r8f_contract_rethrown_error_keeps_identity() {
    let source = r#"
        function main(): void {
            const err = new Error("boom");
            const invoke: any = (cb: any): any => cb();
            try {
                try {
                    invoke(() => { throw err; });
                } catch (e: any) {
                    throw e;
                }
            } catch (outer: any) {
                console.log(outer === err, outer.message);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "r8f_contract_rethrown_error_keeps_identity"),
        "true boom\n"
    );
}

#[test]
fn r8f_contract_plain_string_equality_is_unchanged() {
    let source = r#"
        function main(): void {
            const s: any = "x";
            console.log(s === "x", s === "y");
            const t: any = new Error("q");
            console.log(t === "q", t.name);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "r8f_contract_plain_string_equality_is_unchanged"),
        "true false\nfalse Error\n"
    );
}
