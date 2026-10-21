
// D4: an untyped parameter with a default takes the default's static type.
// Expected output recorded from node 22.

#[test]
fn d4_contract_untyped_default_uses_earlier_parameter_and_literals() {
    let source = r##"function f(a: number, b = a * 2) { return a + b }
function s(a: string, sep = ", ") { return a + sep + "x" }
function t(flag = true, n = 3 + 4) { return flag ? n : -n }
class C { k: number; constructor(x: number, y = x - 1) { this.k = x + y } }
const h = (a: number, b = a * 3, c = b + 1) => a + b + c;
function main(): void {
  console.log(f(1), f(1, 5), s("a"), s("a", "-"), t(), t(false, 2), new C(4).k, h(2), h(2, 1));
}
"##;
    assert_eq!(
        compile_and_run(source, "d4_contract_untyped_default_uses_earlier_parameter_and_literals"),
        "3 6 a, x a-x 7 -2 7 15 5\n"
    );
}
