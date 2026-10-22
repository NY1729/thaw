// Wave HS: `||`/`&&` with a `Json` left operand, and `in` against
// `Object.prototype` keys on a fixed-shape object. Expected output from node 22.

#[test]
fn hs_contract_or_and_with_any_left_keeps_left_value() {
    let source = r####"function f(v: any): void {
  console.log(v || 5);
  console.log(v && 5);
  console.log(v || "x");
  console.log(v && "y");
  console.log(v || {a: 1});
  console.log(v || null);
  console.log(v || true);
  console.log(v && false);
}
async function main(): Promise<void> {
  f("abc");
  f(0);
  f(null);
  f(undefined);
  f(false);
  f(7);
}
"####;
    assert_eq!(
        compile_and_run(source, "hs_contract_or_and_with_any_left_keeps_left_value"),
        "abc\n5\nabc\ny\nabc\nabc\nabc\nfalse\n5\n0\nx\n0\n{ a: 1 }\nnull\ntrue\n0\n5\nnull\nx\nnull\n{ a: 1 }\nnull\ntrue\nnull\n5\nundefined\nx\nundefined\n{ a: 1 }\nnull\ntrue\nundefined\n5\nfalse\nx\nfalse\n{ a: 1 }\nnull\ntrue\nfalse\n7\n5\n7\ny\n7\n7\n7\nfalse\n"
    );
}

#[test]
fn hs_contract_in_operator_sees_object_prototype_keys() {
    let source = r####"async function main(): Promise<void> {
  console.log("toString" in {a: 1});
  console.log("hasOwnProperty" in {a: 1});
  console.log("__proto__" in {a: 1});
  console.log("a" in {a: 1});
  console.log("zz" in {a: 1});
}
"####;
    assert_eq!(
        compile_and_run(source, "hs_contract_in_operator_sees_object_prototype_keys"),
        "true\ntrue\ntrue\ntrue\nfalse\n"
    );
}
