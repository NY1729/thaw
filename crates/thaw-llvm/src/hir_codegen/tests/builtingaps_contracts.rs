// `Date.parse` accepts the common non-ISO spellings V8 does. Only zoned inputs are used so the
// expected values (from node 22) do not depend on the host time zone.

#[test]
fn builtingaps_contract_date_parse_legacy_formats() {
    let source = r#"function main(): void {
  console.log(Date.parse("Tue, 01 Jan 2030 00:00:00 GMT"));
  console.log(Date.parse("Tue Jan 01 2030 00:00:00 GMT+0100"));
  console.log(Date.parse("Jan 1, 2030 10:00 UTC"));
  console.log(Date.parse("1 Jan 2030 3:04 PM GMT"));
  console.log(Date.parse("2030-01-01T00:00:00.5Z"));
  console.log(Date.parse("garbage"));
  console.log(Date.parse("Foo 1, 2030"));
}
"#;
    assert_eq!(
        compile_and_run(source, "builtingaps_contract_date_parse_legacy_formats"),
        "1893456000000\n1893452400000\n1893492000000\n1893510240000\n1893456000500\nNaN\nNaN\n"
    );
}
