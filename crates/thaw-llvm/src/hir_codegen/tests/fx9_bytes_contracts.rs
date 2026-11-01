// `Buffer.prototype.toLocaleString` decodes the bytes as utf8 (it must not take the
// number-array element path that `infer_expr_type` would pick for `Bytes`), while a
// plain number array still formats each element. Expected output recorded from node 22.

#[test]
fn fx9_contract_buffer_to_locale_string_decodes_utf8() {
    let source = r##"function main(): void {
    console.log(Buffer.from("héllo").toLocaleString());
}
"##;
    assert_eq!(
        compile_and_run(source, "fx9_contract_buffer_to_locale_string_decodes_utf8"),
        "héllo
"
    );
}

#[test]
fn fx9_contract_number_array_to_locale_string_still_formats() {
    let source = r##"function main(): void {
    console.log([1234.5, 6].toLocaleString("en-US"));
}
"##;
    assert_eq!(
        compile_and_run(source, "fx9_contract_number_array_to_locale_string_still_formats"),
        "1,234.5,6
"
    );
}
