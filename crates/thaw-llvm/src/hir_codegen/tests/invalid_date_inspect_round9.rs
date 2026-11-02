// Invalid Date inspect: a typed Date whose timestamp is NaN is stored behind
// the `$__thaw_non_finite$` sentinel and must still print as `Invalid Date`.
// Expected output from node 22.

#[test]
fn invalid_date_inspect_round9_console_log_forms() {
    let source = r#"
        function main(): void {
            const d = new Date(NaN);
            console.log(d, [d], { d }, String(d), d.getTime());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "invalid_date_inspect_round9_forms"),
        "Invalid Date [ Invalid Date ] { d: Invalid Date } Invalid Date NaN\n"
    );
}

#[test]
fn invalid_date_inspect_round9_parsed_invalid_string() {
    let source = r#"
        function main(): void {
            console.log(new Date("invalid"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "invalid_date_inspect_round9_parsed"),
        "Invalid Date\n"
    );
}
