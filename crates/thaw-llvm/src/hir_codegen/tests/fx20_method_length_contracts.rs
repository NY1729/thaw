// Class-method `.length` contracts: defaulted and rest parameters are not counted, for instance, static and
// prototype access alike. Expected output from node 22.

#[test]
fn fx20_class_method_length_ignores_defaulted_and_rest_parameters() {
    let source = r#"
class C { m(a: number, b: number = 1) {} static s(a: number, b = 2) {} r(a: number, ...c: number[]) {} }
function main(): void {
  const c = new C();
  console.log(c.m.length, C.s.length, c.r.length, C.prototype.m.length);
}
"#;
    let output = compile_and_run(source, "fx20_class_method_length_ignores_defaulted_and_rest_parameters");
    assert_eq!(output, "1 1 1 1\n");
}

#[test]
fn fx20_class_method_length_for_new_receiver_and_leading_defaults() {
    let source = r#"
class C { m(a: number, b: number = 1) {} static s(a: number, b = 2) {} n(a = 1, b = 2) {} r(a: number, ...c: number[]) {} }
function main(): void {
  console.log(new C().m.length, C.s.length, new C().n.length, new C().r.length);
}
"#;
    let output = compile_and_run(source, "fx20_class_method_length_for_new_receiver_and_leading_defaults");
    assert_eq!(output, "1 1 0 1\n");
}
