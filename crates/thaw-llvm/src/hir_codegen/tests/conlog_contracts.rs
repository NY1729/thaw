// console.log of an expression whose type is only known at the call (array-method result, Date,
// null-prototype object, Buffer element). Expected output from node 22.

#[test]
fn conlog_contract_sort_inline() {
    let source = r#"function main(): void {
  const a: number[] = [3, 1, 2];
  console.log([10, 9, 1, 100].sort());
  console.log(a.toSorted());
  console.log(["b", "a"].sort());
}
"#;
    assert_eq!(
        compile_and_run(source, "conlog_contract_sort_inline"),
        "[ 1, 10, 100, 9 ]\n[ 1, 2, 3 ]\n[ 'a', 'b' ]\n"
    );
}

#[test]
fn conlog_contract_date_inline() {
    let source = r#"function main(): void {
  console.log(new Date(0));
  console.log([new Date(0)]);
}
"#;
    assert_eq!(
        compile_and_run(source, "conlog_contract_date_inline"),
        "1970-01-01T00:00:00.000Z\n[ 1970-01-01T00:00:00.000Z ]\n"
    );
}

#[test]
fn conlog_contract_null_prototype() {
    let source = r#"function main(): void {
  console.log(Object.create(null));
}
"#;
    assert_eq!(
        compile_and_run(source, "conlog_contract_null_prototype"),
        "[Object: null prototype] {}\n"
    );
}

// Node prints an Error's stack, whose first line is "Name: message" (Thaw has no frames).
#[test]
fn conlog_contract_caught_error_prints_name_and_message() {
    let source = r#"function main(): void {
  try {
    throw new Error('boom');
  } catch (e) {
    console.log(e);
  }
  console.log(new Error('boom'));
}
"#;
    let output = compile_and_run(source, "conlog_contract_caught_error");
    assert!(output.starts_with("Error: boom"), "{output:?}");
    assert_eq!(output.lines().filter(|line| *line == "Error: boom").count(), 2, "{output:?}");
}
