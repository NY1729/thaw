// Unary plus on arrays (`+[]`, `+[5]`, `+[1,2]`) and `X.constructor === Array` for array receivers.
// Expected output from node 22.

#[test]
fn unary_plus_array_round9_coerces_through_string_join() {
    let source = r#"function main(): void { console.log(+[], +[5], +[1,2], +["7"]); }"#;
    assert_eq!(
        compile_and_run(source, "unary_plus_array_round9_coerces_through_string_join"),
        "0 5 NaN 7\n"
    );
}

#[test]
fn unary_plus_array_round9_array_constructor_identity() {
    let source = r#"function main(): void {
  const a = [1, 2];
  console.log([].constructor === Array, a.constructor === Array, [].constructor !== Array);
}
"#;
    assert_eq!(
        compile_and_run(source, "unary_plus_array_round9_array_constructor_identity"),
        "true true false\n"
    );
}
