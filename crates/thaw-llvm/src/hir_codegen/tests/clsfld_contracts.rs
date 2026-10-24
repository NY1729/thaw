// Class-instance optional fields print as `undefined` in console.log (like Node); `x!` of an
// absent optional number is NaN. Expected output recorded from node 22.

#[test]
fn clsfld_console_prints_absent_optional_class_fields_as_undefined() {
    let source = r##"class P { o?: number; s?: string; n: number = 1; }
class D extends P { x?: boolean; }
interface I { o?: number }
function main(): void {
  console.log(new P());
  console.log(new D());
  const lit: I = {};
  console.log(lit);
  console.log({ p: new P() });
  console.log(JSON.stringify(new P()));
}
"##;
    assert_eq!(
        compile_and_run(source, "clsfld_console_prints_absent_optional_class_fields_as_undefined"),
        "P { o: undefined, s: undefined, n: 1 }\nD { o: undefined, s: undefined, n: 1, x: undefined }\n{}\n{ p: P { o: undefined, s: undefined, n: 1 } }\n{\"n\":1}\n"
    );
}

#[test]
fn clsfld_non_null_assertion_on_absent_optional_number_is_nan() {
    let source = r##"class P { o?: number }
function main(): void {
  console.log(new P().o! + 1);
  const q = new P();
  q.o = 2;
  console.log(q.o! + 1);
}
"##;
    assert_eq!(compile_and_run(source, "clsfld_non_null_assertion_on_absent_optional_number_is_nan"), "NaN\n3\n");
}
