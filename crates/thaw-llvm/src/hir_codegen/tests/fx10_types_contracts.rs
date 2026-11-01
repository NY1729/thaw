// Round-5 type contracts (fx10): `arguments` alongside a rest parameter, and `.then/.catch`
// on a dynamic receiver. Expected output is from node 22.

#[test]
fn fx10_contract_arguments_with_own_rest_parameter() {
    let source = r#"
function f(a: any, ...r: any[]) {
  return arguments.length;
}
function g(...r: any[]) {
  return arguments[1];
}
function main(): void {
  console.log(f(1, 2, 3), f(), f(1));
  console.log(g("x", "y"));
}
"#;
    assert_eq!(
        compile_and_run(source, "fx10_contract_arguments_with_own_rest_parameter"),
        "3 0 1
y
"
    );
}
