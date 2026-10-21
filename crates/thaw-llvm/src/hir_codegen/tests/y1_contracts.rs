// Wave Y1: `??` on an always-nullish left operand (`undefined`, `null`) yields the right operand.
// Found through a generator whose resume value is `undefined` after `next()` with no argument.
// Expected output from node 22.

#[test]
fn y1_contract_nullish_coalesce_on_undefined_yields_right_operand() {
    let source = r##"function* g() {
  let i = 0;
  while (i < 3) {
    const got = yield i;
    i += got ?? 1;
  }
}
function main(): void {
  const it = g();
  const a = it.next();
  const b = it.next();
  const c = it.next();
  const d = it.next();
  console.log(a.value, a.done, b.value, b.done, c.value, c.done, d.done);
  console.log(undefined ?? 7, null ?? "x");
}
"##;
    assert_eq!(
        compile_and_run(source, "y1_contract_nullish_coalesce_on_undefined_yields_right_operand"),
        "0 false 1 false 2 false true\n7 x\n"
    );
}
