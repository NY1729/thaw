// Round 9: a Json value assigned to an `undefined[]` / `null[]` native array used to fail to
// compile ("unsupported JSON array element Undefined"/"Null"). The array decoder now uses the
// same dummy-bool sentinels as the tuple decoder. Expected output recorded from node 22.

#[test]
fn undefined_json_array_round9_literal_prints_elements() {
    let source = r#"function main(): void {
  const a: undefined[] = [undefined, undefined];
  console.log(a, a.length);
}
"#;
    assert_eq!(
        compile_and_run(source, "undefined_json_array_round9_literal_prints_elements"),
        "[ undefined, undefined ] 2\n"
    );
}

#[test]
fn undefined_json_array_round9_json_decodes_into_undefined_array() {
    let source = r#"function main(): void {
  const j: any = JSON.parse('[1,2]');
  const a: undefined[] = j;
  console.log(a.length);
}
"#;
    assert_eq!(
        compile_and_run(source, "undefined_json_array_round9_json_decodes_into_undefined_array"),
        "2\n"
    );
}

#[test]
fn undefined_json_array_round9_json_decodes_into_null_array() {
    let source = r#"function main(): void {
  const j: any = JSON.parse('[1,2,3]');
  const a: null[] = j;
  console.log(a.length);
}
"#;
    assert_eq!(
        compile_and_run(source, "undefined_json_array_round9_json_decodes_into_null_array"),
        "3\n"
    );
}

#[test]
fn undefined_json_array_round9_native_undefined_array_encodes_to_json() {
    let source = r#"function main(): void {
  const a: undefined[] = [undefined, undefined];
  const j: any = a;
  console.log(j.length, j[0]);
}
"#;
    assert_eq!(
        compile_and_run(source, "undefined_json_array_round9_native_undefined_array_encodes_to_json"),
        "2 undefined\n"
    );
}
